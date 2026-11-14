//! Stages 4 and 5 for a container: its configuration, its data networks or P2p veths, and
//! starting it; then the services a container node runs.

use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::path::Path;

use anyhow::{Context, Result, bail};
use tracing::instrument;

use super::context::Redeploy;
use crate::services::container_networks::{self, ContainerDataInterface};
use crate::services::keycloak;
use crate::services::lab_operations::LabOperation;
use crate::services::node_lifecycle;
use crate::services::node_ops::{self, ContainerZtpResult, LabZtp};
use crate::services::node_secrets::secrets_for_redeploy;
use crate::services::p2p_links::{
    ContainerNode, ContainerVeth, KernelHost, LabLinks, RecreatedEnd, container_veths,
    data_interface_indices, node_bridge_interfaces, node_links, reattach_node_links, report_links,
};
use crate::services::talos;
use crate::services::vault;
use crate::tls::lab_ca::NodeCertPaths;

use container::ContainerSpec;
use shared::data;
use shared::data::{LinkReport, OperationEvent, RecordId, StatusKind};
use shared::konst::{CONTAINER_SECRETS_DIR, SHERPA_MANAGEMENT_NETWORK_NAME, SHERPA_SECRETS_DIR};
use shared::util;

/// A container's data interfaces, and the lab's links they were planned from.
struct DataInterfaces {
    lab_links: LabLinks,
    /// Its Docker data networks, for a container on no link.
    networks: Vec<data::ContainerNetworkAttachment>,
    /// Its veths, for a container on a P2p link.
    veths: Vec<ContainerVeth>,
}

/// Configure the container again, recreate its data networks or P2p veths, start it, re-attach
/// its links, and restore what its services keep (stages 4 and 5). Returns its links' states.
#[instrument(skip_all, fields(lab_id = %redeploy.lab_id, node_name = %redeploy.node_name), level = "debug")]
pub(super) async fn redeploy_container(
    redeploy: &mut Redeploy<'_>,
    cert_paths: &NodeCertPaths,
) -> Result<Vec<LinkReport>> {
    // Read before the node's ZTP takes a custom config out of it.
    let initialises_vault = vault::sherpa_initialises_vault(
        redeploy.target_node.model,
        redeploy.target_node.ztp.config.is_some(),
    );
    let ztp_result = configure_container(redeploy, cert_paths)?;

    let lab_id = redeploy.lab_id.as_str();
    let node_name = redeploy.node_name.as_str();
    let _ = redeploy.progress.send_status(
        format!("Recreating networks and starting container: {node_name}"),
        StatusKind::Progress,
    );

    let container_name = format!("{node_name}-{lab_id}");

    let DataInterfaces {
        lab_links,
        networks: additional_networks,
        veths: p2p_veths,
    } = plan_data_interfaces(redeploy).await?;
    let db = &redeploy.state.db;

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
        &redeploy.state.qemu,
        ContainerSpec {
            name: container_name.clone(),
            image: container::image_reference(&ztp_result.image, &redeploy.node_image.version),
            env_vars: ztp_result.env_vars,
            volumes,
            capabilities: ztp_result.capabilities,
            management_network: mgmt_attachment,
            additional_networks,
            commands: ztp_result.commands,
            privileged: ztp_result.privileged,
            shm_size: ztp_result.shm_size,
            user: ztp_result.user,
            memory_mib: redeploy.db_node.memory.map(u64::from),
            cpu_count: redeploy.db_node.cpu_count,
        },
        redeploy.target_node.model,
        redeploy.progress,
    )
    .await?;

    if !is_running {
        bail!("Container {container_name} is not in running state after redeploy");
    }
    node_lifecycle::record(
        db,
        redeploy.operation,
        &redeploy.node_record_id,
        OperationEvent::Launched,
    )
    .await?;

    // Take the carrier from disabled interfaces, as for VMs.
    if has_disabled {
        let isolated =
            node_ops::node_isolated_network_data(node_name, redeploy.db_node.index, lab_id);
        network::set_link_state(&isolated.bridge_name, false).await?;
    }

    let link_reports = attach_links(redeploy, &lab_links, container_name, p2p_veths).await;

    ready_after_restore(
        db,
        redeploy.operation,
        &redeploy.node_record_id,
        restore_node_services(redeploy, initialises_vault),
    )
    .await?;
    Ok(link_reports)
}

/// Make the redeployed container's veths in its namespace, then attach its links, and report
/// what each link was left in. A container on no link has none.
async fn attach_links(
    redeploy: &Redeploy<'_>,
    lab_links: &LabLinks,
    container_name: String,
    veths: Vec<ContainerVeth>,
) -> Vec<LinkReport> {
    if veths.is_empty() {
        return vec![];
    }
    let host = KernelHost::new(&redeploy.state.docker, &redeploy.state.qemu);
    let outcomes = reattach_node_links(
        &host,
        lab_links,
        &redeploy.node_record_id,
        RecreatedEnd::Container {
            container: container_name,
            veths,
        },
    )
    .await;
    report_links(
        &host,
        lab_links,
        &redeploy.node_record_id,
        Some(redeploy.progress),
        &redeploy.state.link_states,
        outcomes,
    )
    .await
}

/// The data interfaces of a container: Docker networks, made now, for one on no link, or, for
/// one on a P2p link, a veth for every data interface, made once it has started; Docker then
/// only handles its management network.
async fn plan_data_interfaces(redeploy: &Redeploy<'_>) -> Result<DataInterfaces> {
    let db = &redeploy.state.db;
    let lab_links = LabLinks::load(db, &redeploy.lab_id, &redeploy.lab_record_id).await?;
    let links = node_links(lab_links.links(), &redeploy.node_record_id);
    let db_bridges = db::list_bridges(db, &redeploy.lab_record_id).await?;
    let bridge_interfaces = node_bridge_interfaces(&db_bridges, &redeploy.node_record_id);
    let data_indices = data_interface_indices(
        &redeploy.node_image,
        redeploy.data_interface_count,
        &redeploy.node_name,
    )?;
    let (networks, veths) = if links.is_empty() {
        let networks = create_data_networks(redeploy, &bridge_interfaces, data_indices).await?;
        (networks, vec![])
    } else {
        let node = ContainerNode {
            lab_id: &redeploy.lab_id,
            name: &redeploy.node_name,
            index: redeploy.db_node.index,
            model: redeploy.target_node.model,
            record: &redeploy.node_record_id,
        };
        let veths = container_veths(&node, &links, &bridge_interfaces, data_indices)?;
        (vec![], veths)
    };
    Ok(DataInterfaces {
        lab_links,
        networks,
        veths,
    })
}

/// Restore what the node's services keep, then record the node ready. A container whose
/// services were not restored is never recorded running: it stays `Starting` until the
/// redeploy records it failed.
async fn ready_after_restore(
    db: &db::Database,
    operation: &LabOperation,
    node_id: &RecordId,
    restore: impl Future<Output = Result<()>>,
) -> Result<()> {
    restore.await?;
    node_lifecycle::record(db, operation, node_id, OperationEvent::Ready).await?;
    Ok(())
}

/// Generate the container's configuration (stage 4), with the image stage 1 resolved and
/// checked.
fn configure_container(
    redeploy: &mut Redeploy<'_>,
    cert_paths: &NodeCertPaths,
) -> Result<ContainerZtpResult> {
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

#[cfg(test)]
mod tests {
    use anyhow::anyhow;
    use shared::data::NodeState;

    use super::*;
    use crate::services::lab_operations::begin;
    use crate::services::node_lifecycle::tests::lab_of;

    async fn launched_container(lab_id: &str) -> (db::Database, RecordId) {
        let (db, _, nodes) = lab_of(lab_id, 1).await;
        let operation = begin(lab_id).await;
        node_lifecycle::record(&db, &operation, &nodes[0], OperationEvent::Launched)
            .await
            .unwrap();
        (db, nodes[0].clone())
    }

    #[tokio::test]
    async fn a_container_is_not_running_before_its_services_are_restored() {
        // M9.1: redeploy used to record the container running, then restore Vault or Keycloak.
        let (db, node) = launched_container("rd-rest1").await;
        let operation = begin("rd-rest1").await;
        let restore = async {
            let state = db::get_node(&db, node.clone()).await?.state;
            assert_eq!(state, NodeState::Starting);
            Err(anyhow!("Vault did not unseal"))
        };

        let error = ready_after_restore(&db, &operation, &node, restore)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("unseal"));
        assert_eq!(
            db::get_node(&db, node).await.unwrap().state,
            NodeState::Starting
        );
    }

    #[tokio::test]
    async fn a_container_whose_services_are_restored_is_running() {
        let (db, node) = launched_container("rd-rest2").await;
        let operation = begin("rd-rest2").await;

        ready_after_restore(&db, &operation, &node, async { Ok(()) })
            .await
            .unwrap();
        assert_eq!(
            db::get_node(&db, node).await.unwrap().state,
            NodeState::Running
        );
    }
}
