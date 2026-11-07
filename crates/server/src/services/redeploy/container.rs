//! Stages 4 and 5 for a container: its configuration, its data networks or P2p veths, and
//! starting it; then the services a container node runs.

use std::collections::{HashMap, HashSet};
use std::ops::RangeInclusive;
use std::path::Path;

use anyhow::{Context, Result, bail};
use tracing::instrument;

use super::context::{Redeploy, data_interface_indices};
use super::links::{node_bridge_interfaces, node_links, reattach_p2p_redirects};
use crate::services::container_networks::{self, ContainerDataInterface};
use crate::services::keycloak;
use crate::services::node_ops::{self, ContainerZtpResult, LabZtp};
use crate::services::node_secrets::secrets_for_redeploy;
use crate::services::talos;
use crate::services::vault;
use crate::tls::lab_ca::NodeCertPaths;

use container::ContainerSpec;
use shared::data;
use shared::data::{DbLink, NodeState, StatusKind};
use shared::konst::{
    BRIDGE_MEMBER_HOST_VETH_PREFIX, CONTAINER_HOST_VETH_PREFIX, CONTAINER_SECRETS_DIR,
    SHERPA_MANAGEMENT_NETWORK_NAME, SHERPA_SECRETS_DIR,
};
use shared::util;

/// A data interface of a P2p container: a veth created into the container's netns once it has
/// started, with its host end named `host_veth`.
struct P2pContainerVeth {
    host_veth: String,
    host_alias: String,
    interface_idx: u8,
    node_model: data::NodeModel,
    admin_down: bool,
    /// The private bridge the host end joins, for a bridge member.
    bridge: Option<String>,
}

/// Configure the container again, recreate its data networks or P2p veths, start it, and
/// restore what its services keep (stages 4 and 5).
#[instrument(skip_all, fields(lab_id = %redeploy.lab_id, node_name = %redeploy.node_name), level = "debug")]
pub(super) async fn redeploy_container(
    redeploy: &mut Redeploy<'_>,
    cert_paths: &NodeCertPaths,
) -> Result<()> {
    // Read before the node's ZTP takes a custom config out of it.
    let initialises_vault = vault::sherpa_initialises_vault(
        redeploy.target_node.model,
        redeploy.target_node.ztp_config.is_some(),
    );
    let ztp_result = configure_container(redeploy, cert_paths)?;

    let lab_id = redeploy.lab_id.as_str();
    let node_name = redeploy.node_name.as_str();
    let _ = redeploy.progress.send_status(
        format!("Recreating networks and starting container: {node_name}"),
        StatusKind::Progress,
    );

    let container_name = format!("{node_name}-{lab_id}");
    let container_image = format!(
        "{}:{}",
        ztp_result.image,
        redeploy.target_node.version.as_deref().unwrap_or("latest")
    );

    let db = &redeploy.state.db;
    let db_links = db::list_links_by_lab(db, redeploy.lab_record_id.clone()).await?;
    let links = node_links(&db_links, &redeploy.node_record_id);
    let db_bridges = db::list_bridges(db, &redeploy.lab_record_id).await?;
    let bridge_interfaces = node_bridge_interfaces(&db_bridges, &redeploy.node_record_id);
    let data_indices = data_interface_indices(
        &redeploy.node_image,
        redeploy.data_interface_count,
        node_name,
    )?;

    // A container on a P2p link gets a veth for every data interface, created once it has
    // started; Docker only handles its management network.
    let (additional_networks, p2p_veths) = if links.is_empty() {
        let networks = create_data_networks(redeploy, &bridge_interfaces, data_indices).await?;
        (networks, vec![])
    } else {
        let veths = p2p_container_veths(redeploy, &links, &bridge_interfaces, data_indices)?;
        (vec![], veths)
    };

    let mgmt_attachment = data::ContainerNetworkAttachment {
        name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
        ipv4_address: Some(redeploy.ipv4.to_string()),
        ipv6_address: None,
        linux_interface_name: None,
        admin_down: false,
    };

    // Secret files are replaced, removed or kept, and mounted from RAM.
    let mut volumes = ztp_result.volumes;
    if let Some(dir) = secrets_for_redeploy(
        Path::new(SHERPA_SECRETS_DIR),
        lab_id,
        node_name,
        redeploy.secrets.as_deref(),
    )
    .with_context(|| format!("Cannot write secret files for node '{node_name}'"))?
    {
        volumes.push(format!("{}:{CONTAINER_SECRETS_DIR}:ro", dir.display()));
    }

    let has_disabled = additional_networks.iter().any(|n| n.admin_down);
    let is_running = node_ops::start_container_node(
        &redeploy.state.docker,
        ContainerSpec {
            name: container_name.clone(),
            image: container_image,
            env_vars: ztp_result.env_vars,
            volumes,
            capabilities: ztp_result.capabilities,
            management_network: mgmt_attachment,
            additional_networks,
            commands: ztp_result.commands,
            privileged: ztp_result.privileged,
            shm_size: ztp_result.shm_size,
            user: ztp_result.user,
        },
        redeploy.target_node.model,
        redeploy.progress,
    )
    .await?;

    if !is_running {
        bail!("Container {container_name} is not in running state after redeploy");
    }

    // Take the carrier from disabled interfaces, as for VMs.
    if has_disabled {
        let isolated =
            node_ops::node_isolated_network_data(node_name, redeploy.db_node.index, lab_id);
        network::set_link_down(&isolated.bridge_name).await?;
    }

    if !p2p_veths.is_empty() {
        create_p2p_veths(redeploy, &container_name, &p2p_veths).await?;
        reattach_p2p_redirects(lab_id, &links, redeploy.progress).await?;
    }

    db::update_node_state(db, redeploy.node_record_id.clone(), NodeState::Running).await?;
    restore_node_services(redeploy, initialises_vault).await
}

/// Generate the container's configuration (stage 4), with the image the node was created from
/// when the manifest names none, as validated on `sherpa up`.
fn configure_container(
    redeploy: &mut Redeploy<'_>,
    cert_paths: &NodeCertPaths,
) -> Result<ContainerZtpResult> {
    if redeploy.target_node.image.is_none() {
        redeploy.target_node.image = redeploy.node_image.container_repo(None);
    }
    node_ops::generate_container_ztp(
        &mut redeploy.target_node,
        &redeploy.node_image,
        &LabZtp {
            lab_id: &redeploy.lab_id,
            lab_dir: &redeploy.lab_dir,
            images_dir: &redeploy.state.config.images_dir,
            sherpa_user: &redeploy.sherpa_user,
            dns: &redeploy.dns,
            mgmt_net: &redeploy.mgmt_net,
            progress: redeploy.progress,
            owner_account: redeploy.owner_account.as_ref(),
        },
        redeploy.ipv4,
        Some(cert_paths),
    )
}

/// The veths of a container on a P2p link, one for every data interface: its links, its
/// private bridge memberships, whose host end joins the bridge, and its disabled interfaces,
/// admin down.
fn p2p_container_veths(
    redeploy: &Redeploy<'_>,
    links: &[&DbLink],
    bridge_interfaces: &HashMap<String, String>,
    data_indices: RangeInclusive<u8>,
) -> Result<Vec<P2pContainerVeth>> {
    let lab_id = redeploy.lab_id.as_str();
    let node_name = redeploy.node_name.as_str();
    let node_idx = redeploy.db_node.index;
    let model = redeploy.target_node.model;
    let node = &redeploy.node_record_id;
    let mut veths = vec![];

    for link in links {
        let ends = [
            (&link.node_a, &link.int_a, &link.tap_a),
            (&link.node_b, &link.int_b, &link.tap_b),
        ];
        for (end, interface, tap) in ends {
            if end != node {
                continue;
            }
            veths.push(P2pContainerVeth {
                host_veth: tap.clone(),
                host_alias: format!("{lab_id}-p2p-host-{node_name}::{interface}"),
                interface_idx: util::interface_to_idx(&model, interface)?,
                node_model: model,
                admin_down: false,
                bridge: None,
            });
        }
    }

    for (iface_name, bridge_name) in bridge_interfaces {
        let iface_idx = util::interface_to_idx(&model, iface_name)?;
        veths.push(P2pContainerVeth {
            host_veth: format!("{BRIDGE_MEMBER_HOST_VETH_PREFIX}{node_idx}i{iface_idx}-{lab_id}"),
            host_alias: format!("{lab_id}-bridge-host-{node_name}::{iface_name}"),
            interface_idx: iface_idx,
            node_model: model,
            admin_down: false,
            bridge: Some(bridge_name.clone()),
        });
    }

    let connected: HashSet<u8> = veths.iter().map(|v| v.interface_idx).collect();
    for idx in data_indices {
        if connected.contains(&idx) {
            continue;
        }
        veths.push(P2pContainerVeth {
            host_veth: format!("{CONTAINER_HOST_VETH_PREFIX}{node_idx}i{idx}-{lab_id}"),
            host_alias: format!("{lab_id}-p2p-disabled-host-{node_name}::{idx}"),
            interface_idx: idx,
            node_model: model,
            admin_down: true,
            bridge: None,
        });
    }
    Ok(veths)
}

/// Create the Docker macvlan network of each data interface of a container with no links: on
/// its private bridge for a bridge member, otherwise on the node's isolated bridge, disabled.
#[instrument(skip_all, fields(lab_id = %redeploy.lab_id, node_name = %redeploy.node_name), level = "debug")]
async fn create_data_networks(
    redeploy: &Redeploy<'_>,
    bridge_interfaces: &HashMap<String, String>,
    data_indices: RangeInclusive<u8>,
) -> Result<Vec<data::ContainerNetworkAttachment>> {
    let lab_id = redeploy.lab_id.as_str();
    let node_name = redeploy.node_name.as_str();
    let model = &redeploy.target_node.model;
    let docker = &redeploy.state.docker;
    let isolated = node_ops::node_isolated_network_data(node_name, redeploy.db_node.index, lab_id);
    let mut attachments = vec![];

    for idx in data_indices {
        let iface_name = util::interface_from_idx(model, idx)?;
        let connection = if let Some(bridge_name) = bridge_interfaces.get(&iface_name) {
            container::create_docker_macvlan_bridge_network(
                docker,
                bridge_name,
                &container_networks::bridge_member_network_name(node_name, idx, lab_id),
            )
            .await?;
            ContainerDataInterface::Bridged
        } else {
            container::create_docker_macvlan_bridge_network(
                docker,
                &isolated.bridge_name,
                &container_networks::isolated_network_name(node_name, idx, lab_id),
            )
            .await?;
            ContainerDataInterface::Disabled
        };
        attachments.push(container_networks::data_attachment(
            node_name,
            *model,
            &iface_name,
            idx,
            connection,
            lab_id,
        )?);
    }
    Ok(attachments)
}

/// Create the P2p veths into the started container's netns, named and up, so nothing runs in
/// the image, and join bridge members' host ends to their bridge.
#[instrument(skip_all, fields(lab_id = %redeploy.lab_id, %container_name), level = "debug")]
async fn create_p2p_veths(
    redeploy: &Redeploy<'_>,
    container_name: &str,
    veths: &[P2pContainerVeth],
) -> Result<()> {
    let pid = container::get_container_pid(&redeploy.state.docker, container_name).await?;

    for veth_info in veths {
        let target_name =
            util::container_interface_name(&veth_info.node_model, veth_info.interface_idx)?;
        let state = node_ops::netns_veth_state(veth_info.node_model, veth_info.admin_down);

        network::create_veth_in_netns(
            &veth_info.host_veth,
            &veth_info.host_alias,
            &target_name,
            pid,
            state,
        )
        .await
        .with_context(|| {
            format!("Failed to create P2p interface {target_name} in container {container_name}")
        })?;
        if let Some(bridge_name) = &veth_info.bridge {
            network::enslave_to_bridge(&veth_info.host_veth, bridge_name).await?;
        }

        tracing::info!(
            lab_id = %redeploy.lab_id,
            container = %container_name,
            host_veth = %veth_info.host_veth,
            target = %target_name,
            admin_down = veth_info.admin_down,
            "Created P2p veth into container netns (redeploy)"
        );
    }
    Ok(())
}

/// A redeployed Vault starts with empty storage: initialise and unseal it again, and let the
/// clusters that use it log in again. The client's `sherpa download` after the redeploy writes
/// its new credentials. A redeployed Keycloak starts with an empty database: make the realm
/// again, with the users' saved passwords.
#[instrument(skip_all, fields(lab_id = %redeploy.lab_id, node_name = %redeploy.node_name), level = "debug")]
async fn restore_node_services(redeploy: &Redeploy<'_>, initialises_vault: bool) -> Result<()> {
    let lab_id = redeploy.lab_id.as_str();
    let node_name = redeploy.node_name.as_str();
    let ca_cert = &redeploy.lab_ca.pair.cert;
    if initialises_vault {
        let info = vault::ensure_vault_unsealed(
            lab_id,
            node_name,
            redeploy.ipv4,
            ca_cert,
            redeploy.ready_timeout,
            redeploy.progress,
        )
        .await?;
        talos::grant_vault_access(
            &talos::load_clusters(lab_id)?,
            node_name,
            redeploy.ipv4,
            ca_cert,
            &info.root_token,
        )
        .await?;
    }
    if redeploy.target_node.model == data::NodeModel::KeycloakServer {
        keycloak::ensure_keycloak_realm(
            lab_id,
            node_name,
            redeploy.ipv4,
            ca_cert,
            &redeploy.owner_username,
            redeploy.ready_timeout,
            redeploy.progress,
        )
        .await?;
    }
    Ok(())
}
