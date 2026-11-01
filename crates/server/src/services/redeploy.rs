// Server-side implementation of the redeploy operation for a single node

use std::collections::{BTreeMap, HashMap};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use dashmap::DashMap;
use opentelemetry::KeyValue;

use tokio::sync::Mutex;
use tracing::instrument;

use crate::auth::context::AuthContext;
use crate::auth::policy::{require_lab_owner, require_lab_starter};
use crate::daemon::state::AppState;
use crate::services::container_networks::{self, ContainerDataInterface};
use crate::services::guest_account;
use crate::services::image_lifecycle::{lease_image, validate_disk_method};
use crate::services::lab_operations;
use crate::services::node_ops::{self, VmReadiness};
use crate::services::node_resources;
use crate::services::node_secrets::secrets_for_redeploy;
use crate::services::progress::ProgressSender;
use crate::services::ssh_config;
use crate::services::talos;
use crate::services::up::process_manifest_nodes;
use crate::validate::{reject_provisioners, validate_node_secrets};

use shared::data;
use shared::data::{
    DbBridge, DbNode, NodeState, RecordId, RedeployRequest, RedeployResponse, StatusKind,
};
use shared::konst::{
    BRIDGE_MEMBER_HOST_VETH_PREFIX, CONTAINER_SECRETS_DIR, KVM_OUI, LAB_FILE_NAME, MTU_JUMBO_NET,
    READINESS_SLEEP, READINESS_TIMEOUT, SHERPA_LABS_PATH, SHERPA_MANAGEMENT_NETWORK_NAME,
    SHERPA_SECRETS_DIR, TFTP_DIR, ZTP_DIR,
};
use shared::util;
use shared::validators::effective_data_interface_count;
use topology::find_talos_cluster;
use topology::validate::check_performance_tuning;

// Redeploy updates shared lab files. Serialize redeploys within each lab so one
// node's new identity cannot overwrite another node's concurrent update.
static REDEPLOY_LOCKS: OnceLock<DashMap<String, Weak<Mutex<()>>>> = OnceLock::new();

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

/// Redeploy a single node: destroy existing + recreate with fresh ZTP
///
/// Only the lab owner may redeploy, even for admins: the node is rebuilt with the owner's
/// own SSH keys and account. Authorization is checked before any progress is sent or the
/// lab lock is taken. `request.username` is ignored.
#[instrument(skip(ctx, state, progress), fields(username = %ctx.username, lab_id = %request.lab_id, node_name = %request.node_name))]
pub async fn redeploy_node(
    ctx: &AuthContext,
    request: RedeployRequest,
    state: &AppState,
    progress: ProgressSender,
) -> Result<RedeployResponse> {
    let start_time = Instant::now();
    let lab_id = &request.lab_id;
    let node_name = &request.node_name;
    require_lab_starter(ctx)?;
    let db_lab = require_lab_owner(ctx, &state.db, lab_id).await?;
    let owner = db::get_user(&state.db, &ctx.username)
        .await
        .context("Failed to look up the lab owner")?;
    guest_account::require_ssh_key(&owner)?;
    // Destroying the lab stops this operation; see `lab_operations`.
    let operation = lab_operations::begin(lab_id).await;
    let locks = REDEPLOY_LOCKS.get_or_init(DashMap::new);
    locks.retain(|_, lock| lock.strong_count() > 0);
    let lab_lock = {
        let mut entry = locks.entry(lab_id.clone()).or_default();
        match entry.upgrade() {
            Some(lock) => lock,
            None => {
                let lock = Arc::new(Mutex::new(()));
                *entry = Arc::downgrade(&lock);
                lock
            }
        }
    };
    let _lab_guard = lab_lock.lock().await;

    tracing::info!(
        lab_id = %lab_id,
        node_name = %node_name,
        "Starting node redeploy"
    );

    // ========================================================================
    // Stage 1: Load context
    // ========================================================================
    let _ = progress.send_status(
        format!("Loading context for node: {}", node_name),
        StatusKind::Progress,
    );

    // Parse manifest
    let manifest: topology::Manifest =
        serde_json::from_value(request.manifest).context("Failed to deserialize manifest")?;
    reject_provisioners(&manifest).context("Manifest validation failed: provisioners")?;
    if let Some(files) = request.secrets.as_deref()
        && !files.is_empty()
    {
        validate_node_secrets(
            &manifest,
            &BTreeMap::from([(node_name.clone(), files.to_vec())]),
        )
        .context("Manifest validation failed: container secret files")?;
    }

    let manifest_nodes = process_manifest_nodes(&manifest.nodes);

    let mut target_node = manifest_nodes
        .iter()
        .find(|n| n.name == *node_name)
        .ok_or_else(|| anyhow!("Node '{}' not found in manifest", node_name))?
        .clone();

    // Decode base64 ztp_config if present
    if let Some(ref encoded) = target_node.ztp_config {
        let decoded = util::base64_decode(encoded)
            .with_context(|| format!("Failed to decode ztp_config for node '{}'", node_name))?;
        target_node.ztp_config = Some(decoded);
    }
    let talos_membership = find_talos_cluster(
        manifest.talos_clusters.as_deref().unwrap_or_default(),
        node_name,
    );
    talos::ensure_redeployable(node_name, talos_membership)?;

    // Use the shared database connection from AppState
    let db = state.db.clone();

    let lab_record_id = db::get_lab_id(&db_lab).context("Failed to get lab record ID")?;

    // Get node from DB
    let db_node = db::get_node_by_name_and_lab(&db, node_name, lab_record_id.clone())
        .await
        .context(format!(
            "Node '{}' not found in database for lab '{}'",
            node_name, lab_id
        ))?;

    let node_record_id = db::get_node_id(&db_node).context("Failed to get node record ID")?;

    // Get node image config
    let node_images = db::list_node_images_by_ids(&db, vec![db_node.image.clone()])
        .await
        .context("Failed to get node image config")?;

    let node_image = node_images
        .first()
        .ok_or_else(|| anyhow!("Node image config not found for node '{}'", node_name))?
        .clone();
    ensure!(
        target_node.model == node_image.model,
        "Redeploy cannot change node model; expected {}",
        node_image.model
    );
    let data_interface_count =
        effective_data_interface_count(node_name, target_node.data_interface_count, &node_image)
            .context(format!(
                "Data interface count validation failed for node: {}",
                node_name
            ))?;

    let image_guard = lease_image(node_image.model, &node_image.version).await;
    target_node.version = Some(node_image.version.clone());
    validate_disk_method(
        target_node.boot_disk_clone_method,
        &node_image,
        &state.config.images_dir,
        target_node.boot_disk_size,
    )?;

    // Load lab info from filesystem
    let lab_dir = format!("{SHERPA_LABS_PATH}/{lab_id}");
    let lab_file = util::load_file(&format!("{lab_dir}/{LAB_FILE_NAME}"))
        .context("Unable to load lab file")?;
    let mut lab_info: data::LabInfo = lab_file.parse().context("Failed to parse lab info file")?;

    let management_network = format!("{}-{}", SHERPA_MANAGEMENT_NETWORK_NAME, lab_id);
    let mgmt_net = data::SherpaNetwork {
        v4: data::NetworkV4 {
            prefix: lab_info.ipv4_network,
            first: lab_info.ipv4_gateway,
            last: lab_info.ipv4_network.broadcast(),
            boot_server: lab_info.ipv4_router,
            network: lab_info.ipv4_network.network(),
            subnet_mask: lab_info.ipv4_network.netmask(),
            hostmask: lab_info.ipv4_network.hostmask(),
            prefix_length: lab_info.ipv4_network.prefix_len(),
        },
        v6: match (
            lab_info.ipv6_network,
            lab_info.ipv6_gateway,
            lab_info.ipv6_router,
        ) {
            (Some(v6_net), Some(v6_gw), Some(v6_rtr)) => Some(data::NetworkV6 {
                prefix: v6_net,
                first: v6_gw,
                last: util::get_ipv6_addr(&v6_net, u32::MAX).unwrap_or(v6_net.network()),
                boot_server: v6_rtr,
                network: v6_net.network(),
                prefix_length: v6_net.prefix_len(),
            }),
            _ => None,
        },
    };
    let dns = if let Some(ref v6) = mgmt_net.v6 {
        util::default_dns_dual_stack(&lab_info.ipv4_network, &v6.prefix)?
    } else {
        util::default_dns(&lab_info.ipv4_network)?
    };
    // A cluster worker rejoins with a config built from the cluster's saved secrets.
    if let Some((talos_cluster, _)) = talos_membership {
        talos::secrets::ensure_supported_version(node_name, &node_image.version)?;
        let cluster = talos::load_cluster(lab_id, &talos_cluster.name)?;
        target_node.ztp_config = Some(talos::node_machine_config(
            &cluster,
            node_name,
            &mgmt_net.v4,
        )?);
    }
    let sherpa_user = util::sherpa_user().context("Failed to get sherpa user")?;
    let node_idx = db_node.index;
    let (node_ipv4_address, node_ipv6_address) = node_management_addresses(&db_node, &mgmt_net)?;
    target_node.ipv6_address = node_ipv6_address;

    let personal_account = if manifest.provision_current_user
        && guest_account::supports_guest_account(&node_image)
    {
        guest_account::validate_node(&target_node, &node_image)?;
        let account = guest_account::resolve_owner(&db, db_lab.user.clone()).await?;
        if account.is_none() {
            let _ = progress.send_status(
                    format!(
                        "Username '{}' cannot be a login on lab nodes; '{node_name}' uses the sherpa account",
                        ctx.username
                    ),
                    StatusKind::Info,
                );
        }
        account
    } else {
        None
    };
    // Calculate the next SSH config before teardown, preserving every other node's identity.
    lab_info.set_ssh_username(
        node_name,
        personal_account
            .as_ref()
            .map(|account| account.username.as_str()),
    );
    // The rebuilt node gets a new host key, recorded again once it is ready.
    db::update_node_host_key(&db, node_record_id.clone(), None).await?;

    // Update node state to Starting
    db::update_node_state(&db, node_record_id.clone(), NodeState::Starting).await?;

    let _ = progress.send_status(
        format!("Context loaded for node: {}", node_name),
        StatusKind::Done,
    );

    // Get connections
    let docker_conn = state.docker.clone();
    let qemu_conn = Arc::new(
        state
            .qemu
            .connect()
            .context("Failed to connect to libvirt")?,
    );

    // The node's performance tuning must still fit the server before the node is destroyed.
    check_performance_tuning(&target_node, &node_image).context(format!(
        "Performance tuning validation failed for node: {node_name}"
    ))?;
    node_resources::check_cpu_pins_on_server(
        &db,
        &qemu_conn,
        &[&target_node],
        Some(&node_record_id),
    )
    .await
    .context("CPU pinning validation failed")?;

    // ========================================================================
    // Stage 2: Destroy existing node
    // ========================================================================
    let _ = progress.send_status(
        format!("Destroying existing node: {}", node_name),
        StatusKind::Progress,
    );

    match node_image.kind {
        data::NodeKind::VirtualMachine => {
            node_ops::destroy_vm_node(
                qemu_conn.clone(),
                node_name,
                lab_id,
                node_idx,
                node_image.reserved_interface_count,
            )
            .await?;
        }
        data::NodeKind::Container => {
            node_ops::destroy_container_node(&docker_conn, node_name, lab_id).await?;
        }
        data::NodeKind::Unikernel => {
            bail!("Redeploy not supported for unikernel nodes");
        }
    }

    // Delete node's ZTP config dir
    let node_ztp_dir = format!("{lab_dir}/{node_name}");
    if std::path::Path::new(&node_ztp_dir).exists() {
        std::fs::remove_dir_all(&node_ztp_dir)
            .with_context(|| format!("Failed to remove ZTP dir: {}", node_ztp_dir))?;
    }

    let _ = progress.send_status(
        format!("Existing node destroyed: {}", node_name),
        StatusKind::Done,
    );

    // ========================================================================
    // Stage 3: Recreate per-node networks (VM only)
    // ========================================================================
    if matches!(node_image.kind, data::NodeKind::VirtualMachine) {
        let _ = progress.send_status(
            format!("Recreating networks for node: {}", node_name),
            StatusKind::Progress,
        );

        let isolated = node_ops::node_isolated_network_data(node_name, node_idx, lab_id);
        let iso_network = libvirt::IsolatedNetwork {
            network_name: isolated.network_name,
            bridge_name: isolated.bridge_name,
        };
        iso_network.create(&qemu_conn)?;

        if node_image.reserved_interface_count > 0 {
            let reserved = node_ops::node_reserved_network_data(node_name, node_idx, lab_id);
            let res_network = libvirt::ReservedNetwork {
                network_name: reserved.network_name,
                bridge_name: reserved.bridge_name,
            };
            res_network.create(&qemu_conn)?;
        }

        let _ = progress.send_status(
            format!("Networks recreated for node: {}", node_name),
            StatusKind::Done,
        );
    }

    // ========================================================================
    // Stage 4: Regenerate ZTP
    // ========================================================================
    let _ = progress.send_status(
        format!("Regenerating ZTP for node: {}", node_name),
        StatusKind::Progress,
    );

    let config = state.config.clone();

    // The node keeps its VNC port; its vCPUs and memory follow the manifest.
    let db_node = node_resources::reserve_node_resources(
        &db,
        &db_node,
        &node_image.kind,
        node_resources::planned_resources(&target_node, &node_image),
    )
    .await?;

    match node_image.kind {
        data::NodeKind::Container => {
            // As validated on `sherpa up`, from the image the node was created from.
            if target_node.image.is_none() {
                target_node.image = node_image.container_repo(None);
            }
            let ztp_result = node_ops::generate_container_ztp(
                &mut target_node,
                &node_image,
                &lab_dir,
                &sherpa_user,
                &dns,
                &mgmt_net,
                node_ipv4_address,
                &progress,
                None,
            )?;

            // Update the target_node fields from ZTP result
            target_node.image = Some(ztp_result.image.clone());
            target_node.environment_variables = Some(ztp_result.env_vars.clone());
            target_node.commands = Some(ztp_result.commands.clone());
            target_node.privileged = Some(ztp_result.privileged);
            if ztp_result.user.is_some() {
                target_node.user = ztp_result.user.clone();
            }

            // Stage 5: Recreate Docker networks and start container
            let _ = progress.send_status(
                format!("Recreating networks and starting container: {}", node_name),
                StatusKind::Progress,
            );

            let container_name = format!("{}-{}", node_name, lab_id);
            let container_image = format!(
                "{}:{}",
                ztp_result.image,
                target_node.version.as_deref().unwrap_or("latest")
            );

            // Build interface-to-docker-network lookup from DB links
            let db_links = db::list_links_by_lab(&db, lab_record_id.clone()).await?;
            let db_bridges = db::list_bridges(&db, &lab_record_id).await?;
            let bridge_interfaces = node_bridge_interfaces(&db_bridges, &node_record_id);

            // Check if this node has any P2p links
            let is_p2p_container = db_links
                .iter()
                .any(|link| link.node_a == node_record_id || link.node_b == node_record_id);

            let first_data_idx = node_image
                .reserved_interface_count
                .checked_add(1)
                .ok_or_else(|| {
                    anyhow!("Reserved interface count overflow for node {}", node_name)
                })?;
            let max_iface_idx = node_image
                .reserved_interface_count
                .checked_add(data_interface_count)
                .ok_or_else(|| anyhow!("Data interface count overflow for node {}", node_name))?;

            let mut additional_networks = vec![];
            let mut p2p_container_veths: Vec<P2pContainerVeth> = vec![];

            if is_p2p_container {
                // P2p container: a veth for ALL data interfaces (linked + disabled), created
                // once the container has started. Docker only handles the management network.

                // P2p linked interfaces
                for link in &db_links {
                    if link.node_a == node_record_id {
                        let iface_idx = util::interface_to_idx(&target_node.model, &link.int_a)?;
                        p2p_container_veths.push(P2pContainerVeth {
                            host_veth: link.tap_a.clone(),
                            host_alias: format!(
                                "{}-p2p-host-{}::{}",
                                lab_id, node_name, link.int_a
                            ),
                            interface_idx: iface_idx,
                            node_model: target_node.model,
                            admin_down: false,
                            bridge: None,
                        });
                    }
                    if link.node_b == node_record_id {
                        let iface_idx = util::interface_to_idx(&target_node.model, &link.int_b)?;
                        p2p_container_veths.push(P2pContainerVeth {
                            host_veth: link.tap_b.clone(),
                            host_alias: format!(
                                "{}-p2p-host-{}::{}",
                                lab_id, node_name, link.int_b
                            ),
                            interface_idx: iface_idx,
                            node_model: target_node.model,
                            admin_down: false,
                            bridge: None,
                        });
                    }
                }

                // Bridge members; the host end joins the bridge
                for (iface_name, bridge_name) in &bridge_interfaces {
                    let iface_idx = util::interface_to_idx(&target_node.model, iface_name)?;
                    p2p_container_veths.push(P2pContainerVeth {
                        host_veth: format!(
                            "{BRIDGE_MEMBER_HOST_VETH_PREFIX}{node_idx}i{iface_idx}-{lab_id}"
                        ),
                        host_alias: format!("{}-bridge-host-{}::{}", lab_id, node_name, iface_name),
                        interface_idx: iface_idx,
                        node_model: target_node.model,
                        admin_down: false,
                        bridge: Some(bridge_name.clone()),
                    });
                }

                // Disabled interfaces
                let linked_iface_idxs: std::collections::HashSet<u8> = p2p_container_veths
                    .iter()
                    .map(|v| v.interface_idx)
                    .collect();
                for idx in first_data_idx..=max_iface_idx {
                    if linked_iface_idxs.contains(&idx) {
                        continue;
                    }
                    p2p_container_veths.push(P2pContainerVeth {
                        host_veth: format!("cd{}i{}-{}", node_idx, idx, lab_id),
                        host_alias: format!("{}-p2p-disabled-host-{}::{}", lab_id, node_name, idx),
                        interface_idx: idx,
                        node_model: target_node.model,
                        admin_down: true,
                        bridge: None,
                    });
                }
            } else {
                // A container with no links: Docker macvlan networks for bridge members and
                // disabled interfaces
                let isolated = node_ops::node_isolated_network_data(node_name, node_idx, lab_id);

                for idx in first_data_idx..=max_iface_idx {
                    let iface_name = util::interface_from_idx(&target_node.model, idx)?;

                    let connection = if let Some(bridge_name) = bridge_interfaces.get(&iface_name) {
                        container::create_docker_macvlan_bridge_network(
                            &docker_conn,
                            bridge_name,
                            &container_networks::bridge_member_network_name(node_name, idx, lab_id),
                        )
                        .await?;
                        ContainerDataInterface::Bridged
                    } else {
                        container::create_docker_macvlan_bridge_network(
                            &docker_conn,
                            &isolated.bridge_name,
                            &container_networks::isolated_network_name(node_name, idx, lab_id),
                        )
                        .await?;
                        ContainerDataInterface::Disabled
                    };
                    additional_networks.push(container_networks::data_attachment(
                        node_name,
                        &target_node.model,
                        &iface_name,
                        idx,
                        connection,
                        lab_id,
                    )?);
                }
            }

            let mgmt_attachment = data::ContainerNetworkAttachment {
                name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
                ipv4_address: Some(node_ipv4_address.to_string()),
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
                request.secrets.as_deref(),
            )
            .with_context(|| format!("Cannot write secret files for node '{node_name}'"))?
            {
                volumes.push(format!("{}:{CONTAINER_SECRETS_DIR}:ro", dir.display()));
            }

            let has_disabled = additional_networks.iter().any(|n| n.admin_down);
            let is_running = node_ops::start_container_node(
                &docker_conn,
                &container_name,
                &container_image,
                ztp_result.env_vars,
                volumes,
                ztp_result.capabilities,
                mgmt_attachment,
                additional_networks,
                ztp_result.commands,
                ztp_result.privileged,
                ztp_result.shm_size,
                ztp_result.user,
                target_node.model,
                &progress,
            )
            .await?;

            if !is_running {
                bail!(
                    "Container {} is not in running state after redeploy",
                    container_name
                );
            }

            // Take the carrier from disabled interfaces, as for VMs.
            if has_disabled {
                let isolated = node_ops::node_isolated_network_data(node_name, node_idx, lab_id);
                network::set_link_down(&isolated.bridge_name).await?;
            }

            // P2p post-start: create veths into the container's netns, named and up, so
            // nothing runs in the image, then attach eBPF
            if is_p2p_container && !p2p_container_veths.is_empty() {
                let pid = container::get_container_pid(&docker_conn, &container_name).await?;

                for veth_info in &p2p_container_veths {
                    let target_name = if veth_info.node_model == data::NodeModel::NokiaSrlinux {
                        let iface_name = util::interface_from_idx(
                            &veth_info.node_model,
                            veth_info.interface_idx,
                        )?;
                        util::srlinux_to_linux_interface(&iface_name)?
                    } else {
                        format!("eth{}", veth_info.interface_idx)
                    };

                    network::create_veth_in_netns(
                        &veth_info.host_veth,
                        &veth_info.host_alias,
                        &target_name,
                        pid,
                        !veth_info.admin_down,
                    )
                    .await
                    .with_context(|| {
                        format!(
                            "Failed to create P2p interface {target_name} in container \
                             {container_name}"
                        )
                    })?;
                    if let Some(bridge_name) = &veth_info.bridge {
                        network::enslave_to_bridge(&veth_info.host_veth, bridge_name).await?;
                    }

                    tracing::info!(
                        lab_id = %lab_id,
                        container = %container_name,
                        host_veth = %veth_info.host_veth,
                        target = %target_name,
                        admin_down = veth_info.admin_down,
                        "Created P2p veth into container netns (redeploy)"
                    );
                }

                // Re-attach eBPF redirect on P2p links involving this node
                for link in &db_links {
                    if link.node_a != node_record_id && link.node_b != node_record_id {
                        continue;
                    }

                    let ifindex_a = network::get_ifindex(&link.tap_a)
                        .await
                        .context(format!("failed to get ifindex for {}", link.tap_a))?;
                    let ifindex_b = network::get_ifindex(&link.tap_b)
                        .await
                        .context(format!("failed to get ifindex for {}", link.tap_b))?;

                    network::attach_p2p_redirect(&link.tap_a, ifindex_b)
                        .context(format!("failed to attach eBPF redirect on {}", link.tap_a))?;
                    network::attach_p2p_redirect(&link.tap_b, ifindex_a)
                        .context(format!("failed to attach eBPF redirect on {}", link.tap_b))?;

                    network::apply_link_netem(
                        ifindex_a,
                        ifindex_b,
                        &link.impairment_a_to_b,
                        &link.impairment_b_to_a,
                    )
                    .await?;

                    tracing::info!(
                        lab_id = %lab_id,
                        tap_a = %link.tap_a,
                        tap_b = %link.tap_b,
                        "Re-attached eBPF P2p redirect (redeploy)"
                    );
                }
            }

            // Update node state
            db::update_node_state(&db, node_record_id, NodeState::Running).await?;
        }
        data::NodeKind::VirtualMachine => {
            let ztp_dir = format!("{lab_dir}/{ZTP_DIR}");
            let tftp_dir = format!("{ztp_dir}/{TFTP_DIR}");

            // Ensure ZTP directories exist
            util::create_dir(&ztp_dir)?;
            util::create_dir(&tftp_dir)?;

            let vm_ztp = node_ops::generate_vm_ztp(
                &mut target_node,
                &node_image,
                lab_id,
                &lab_dir,
                &tftp_dir,
                &config.images_dir,
                &mgmt_net,
                node_ipv4_address,
                &sherpa_user,
                &dns,
                &progress,
                db_node.mgmt_mac.as_deref(),
                None,
                personal_account.as_ref(),
            )?;

            let _ = progress.send_status(
                format!("ZTP regenerated for node: {}", node_name),
                StatusKind::Done,
            );

            // Stage 5: Clone disks and create VM
            let _ = progress.send_status(
                format!("Cloning disks for node: {}", node_name),
                StatusKind::Progress,
            );

            node_ops::clone_node_disks(
                qemu_conn.clone(),
                vm_ztp.clone_disks,
                lab_id,
                &progress,
                vec![image_guard.clone()],
            )
            .await?;

            // Build interfaces from DB links
            let loopback_subnet = lab_info.loopback_network;
            let db_links = db::list_links_by_lab(&db, lab_record_id.clone()).await?;

            // Build the node setup data to reconstruct interfaces
            let _all_node_images = db::list_node_images(&db).await?;

            let mut interfaces: Vec<data::Interface> = vec![];

            // Management interface
            interfaces.push(data::Interface {
                name: util::dasher(&node_image.management_interface.to_string()),
                num: 0,
                mtu: node_image.interface_mtu,
                mac_address: vm_ztp.mac_address.clone(),
                connection_type: data::ConnectionTypes::Management,
            });

            // Reserved interfaces
            let first_data_interface_idx = node_image
                .reserved_interface_count
                .checked_add(1)
                .ok_or_else(|| {
                    anyhow!("Reserved interface count overflow for node {}", node_name)
                })?;
            for idx in 1..first_data_interface_idx {
                interfaces.push(data::Interface {
                    name: format!("int{}", idx),
                    num: idx,
                    mtu: node_image.interface_mtu,
                    mac_address: util::random_mac(KVM_OUI),
                    connection_type: data::ConnectionTypes::Reserved,
                });
            }

            // Data interfaces
            let max_interface_idx = node_image
                .reserved_interface_count
                .checked_add(data_interface_count)
                .ok_or_else(|| anyhow!("Data interface count overflow for node {}", node_name))?;
            let db_bridges = db::list_bridges(&db, &lab_record_id).await?;
            let bridge_interfaces = node_bridge_interfaces(&db_bridges, &node_record_id);
            for idx in first_data_interface_idx..=max_interface_idx {
                let interface_name = util::interface_from_idx(&target_node.model, idx)?;

                // Check if this interface has a link
                let mut found_link = false;
                for link in &db_links {
                    if link.node_a == node_record_id && link.int_a == interface_name {
                        interfaces.push(data::Interface {
                            name: link.tap_a.clone(),
                            num: idx,
                            mtu: MTU_JUMBO_NET,
                            mac_address: util::random_mac(KVM_OUI),
                            connection_type: data::ConnectionTypes::P2p,
                        });
                        found_link = true;
                        break;
                    }
                    if link.node_b == node_record_id && link.int_b == interface_name {
                        interfaces.push(data::Interface {
                            name: link.tap_b.clone(),
                            num: idx,
                            mtu: MTU_JUMBO_NET,
                            mac_address: util::random_mac(KVM_OUI),
                            connection_type: data::ConnectionTypes::P2p,
                        });
                        found_link = true;
                        break;
                    }
                }

                // Check bridges
                if !found_link {
                    if let Some(bridge_name) = bridge_interfaces.get(&interface_name) {
                        interfaces.push(data::Interface {
                            name: bridge_name.clone(),
                            num: idx,
                            mtu: node_image.interface_mtu,
                            mac_address: util::random_mac(KVM_OUI),
                            connection_type: data::ConnectionTypes::PrivateBridge,
                        });
                    } else {
                        // Disabled interface - connected to isolated network
                        interfaces.push(data::Interface {
                            name: util::dasher(&util::interface_from_idx(&target_node.model, idx)?),
                            num: idx,
                            mtu: node_image.interface_mtu,
                            mac_address: util::random_mac(KVM_OUI),
                            connection_type: data::ConnectionTypes::Disabled,
                        });
                    }
                }
            }

            // Build network names
            let isolated_net = node_ops::node_isolated_network_data(node_name, node_idx, lab_id);
            let reserved_network = if node_image.reserved_interface_count > 0 {
                node_ops::node_reserved_network_data(node_name, node_idx, lab_id).network_name
            } else {
                String::new()
            };

            let has_disabled = interfaces
                .iter()
                .any(|i| matches!(i.connection_type, data::ConnectionTypes::Disabled));

            let domain = node_ops::build_domain_template(
                &target_node,
                &node_image,
                lab_id,
                &config.qemu_bin,
                vm_ztp.disks,
                interfaces,
                vm_ztp.qemu_commands,
                util::get_ip(&loopback_subnet, node_idx as u8).to_string(),
                management_network.clone(),
                isolated_net.network_name,
                reserved_network,
                &node_resources::resources_of(&db_node),
            )?;

            let _ =
                progress.send_status(format!("Creating VM: {}", node_name), StatusKind::Progress);

            node_ops::create_vm(qemu_conn.clone(), domain, &progress).await?;

            // Re-attach eBPF redirect on P2p links involving this VM
            let p2p_vm_links: Vec<_> = db_links
                .iter()
                .filter(|l| l.node_a == node_record_id || l.node_b == node_record_id)
                .collect();

            if !p2p_vm_links.is_empty() {
                let _ = progress.send_status(
                    format!(
                        "Re-attaching eBPF redirect for {} P2p links",
                        p2p_vm_links.len()
                    ),
                    StatusKind::Progress,
                );

                for link in &p2p_vm_links {
                    let ifindex_a = network::get_ifindex(&link.tap_a)
                        .await
                        .context(format!("failed to get ifindex for {}", link.tap_a))?;
                    let ifindex_b = network::get_ifindex(&link.tap_b)
                        .await
                        .context(format!("failed to get ifindex for {}", link.tap_b))?;

                    network::attach_p2p_redirect(&link.tap_a, ifindex_b)
                        .context(format!("failed to attach eBPF redirect on {}", link.tap_a))?;
                    network::attach_p2p_redirect(&link.tap_b, ifindex_a)
                        .context(format!("failed to attach eBPF redirect on {}", link.tap_b))?;

                    network::apply_link_netem(
                        ifindex_a,
                        ifindex_b,
                        &link.impairment_a_to_b,
                        &link.impairment_b_to_a,
                    )
                    .await?;

                    tracing::info!(
                        lab_id = %lab_id,
                        tap_a = %link.tap_a,
                        tap_b = %link.tap_b,
                        "Re-attached eBPF P2p redirect (VM redeploy)"
                    );
                }
            }

            // Set isolated bridge DOWN to remove carrier from disabled VM interfaces
            if has_disabled {
                network::set_link_down(&isolated_net.bridge_name).await?;
            }

            // Stage 6: Readiness check
            let _ = progress.send_status(
                format!("Waiting for node {} to become ready", node_name),
                StatusKind::Waiting,
            );

            let ready_timeout =
                Duration::from_secs(manifest.ready_timeout.unwrap_or(READINESS_TIMEOUT));
            let ready_start = Instant::now();
            let ip_str = node_ipv4_address.to_string();
            let mut is_ready = false;

            let skip_ready = target_node.skip_ready_check.unwrap_or(false);
            if skip_ready {
                let _ = progress.send_status(
                    format!("Node {} - Ready check skipped", node_name),
                    StatusKind::Done,
                );
                is_ready = true;
            }

            let ready_port = node_ops::vm_ready_port(&target_node.model, target_node.ready_port);
            let ready_label = node_ops::ready_port_label(ready_port);
            while !is_ready && ready_start.elapsed() < ready_timeout {
                match node_ops::check_vm_ready(&ip_str, ready_port).await? {
                    VmReadiness::Ready { host_key } => {
                        if host_key.is_some() {
                            db::update_node_host_key(
                                &db,
                                node_record_id.clone(),
                                host_key.as_deref(),
                            )
                            .await?;
                        }
                        is_ready = true;
                        let _ = progress.send_status(
                            format!("Node {} - Ready ({ready_label} reachable)", node_name),
                            StatusKind::Done,
                        );
                    }
                    VmReadiness::NotReady => {
                        let _ = progress.send_status(
                            format!("Node {} - Waiting for {ready_label}", node_name),
                            StatusKind::Waiting,
                        );
                        operation
                            .sleep(Duration::from_secs(READINESS_SLEEP))
                            .await?;
                    }
                }
            }

            if is_ready {
                db::update_node_state(&db, node_record_id, NodeState::Running).await?;
            } else {
                let _ = progress.send_status(
                    format!("Node {} did not become ready within timeout", node_name),
                    StatusKind::Waiting,
                );
            }
        }
        data::NodeKind::Unikernel => {
            bail!("Redeploy not supported for unikernel nodes");
        }
    }

    operation.ensure_active()?;
    ssh_config::refresh_lab_ssh_files(
        &db,
        &lab_dir,
        &lab_record_id,
        &lab_info,
        state.config.server_ipv4,
    )
    .await
    .context("Node redeployed, but failed to save the lab SSH files")?;

    let total_time = start_time.elapsed().as_secs();

    state.metrics.operation_duration.record(
        start_time.elapsed().as_secs_f64(),
        &[KeyValue::new("operation.type", "redeploy")],
    );

    let response = RedeployResponse {
        success: true,
        node_name: node_name.to_string(),
        message: format!("Node '{}' redeployed successfully", node_name),
        total_time_secs: total_time,
    };

    tracing::info!(
        lab_id = %lab_id,
        node_name = %node_name,
        total_time_secs = total_time,
        "Node redeploy completed"
    );

    Ok(response)
}

/// The node's management addresses. Saved addresses win, so a node keeps the address the
/// lab's dnsmasq and SSH config already hold, even if the address layout has changed since
/// the lab was deployed. Nodes without saved addresses get them from their index.
fn node_management_addresses(
    db_node: &DbNode,
    mgmt_net: &data::SherpaNetwork,
) -> Result<(Ipv4Addr, Option<Ipv6Addr>)> {
    let ipv4 = match db_node.mgmt_ipv4.as_deref() {
        Some(saved) => saved
            .parse()
            .with_context(|| format!("Invalid saved management address: {saved}"))?,
        None => util::get_ipv4_addr(
            &mgmt_net.v4.prefix,
            util::node_management_host_offset(db_node.index)?,
        )?,
    };
    let ipv6 = match (db_node.mgmt_ipv6.as_deref(), mgmt_net.v6.as_ref()) {
        (_, None) => None,
        (Some(saved), Some(_)) => Some(
            saved
                .parse()
                .with_context(|| format!("Invalid saved IPv6 management address: {saved}"))?,
        ),
        (None, Some(v6)) => Some(util::get_ipv6_addr(
            &v6.prefix,
            util::node_management_host_offset(db_node.index)?,
        )?),
    };
    Ok((ipv4, ipv6))
}

/// The node's interfaces in private bridges, mapped to the bridge's Linux bridge name.
fn node_bridge_interfaces(bridges: &[DbBridge], node: &RecordId) -> HashMap<String, String> {
    let mut interfaces = HashMap::new();
    for bridge in bridges {
        for member in &bridge.members {
            if member.node == *node {
                interfaces.insert(member.interface.clone(), bridge.bridge_name.clone());
            }
        }
    }
    interfaces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mgmt_net(with_v6: bool) -> data::SherpaNetwork {
        let prefix = util::get_ipv4_network("172.31.1.0/24").unwrap();
        let v6_prefix = util::get_ipv6_network("fd00:b00b:0:1::/64").unwrap();
        let v6 = if with_v6 {
            Some(data::NetworkV6 {
                prefix: v6_prefix,
                first: "fd00:b00b:0:1::1".parse().unwrap(),
                last: "fd00:b00b:0:1:ffff:ffff:ffff:ffff".parse().unwrap(),
                boot_server: "fd00:b00b:0:1::2".parse().unwrap(),
                network: v6_prefix.network(),
                prefix_length: v6_prefix.prefix_len(),
            })
        } else {
            None
        };
        data::SherpaNetwork {
            v4: data::NetworkV4 {
                prefix,
                first: "172.31.1.1".parse().unwrap(),
                last: prefix.broadcast(),
                boot_server: "172.31.1.2".parse().unwrap(),
                network: prefix.network(),
                subnet_mask: prefix.netmask(),
                hostmask: prefix.hostmask(),
                prefix_length: prefix.prefix_len(),
            },
            v6,
        }
    }

    fn db_node(index: u16, mgmt_ipv4: Option<&str>, mgmt_ipv6: Option<&str>) -> DbNode {
        DbNode {
            id: None,
            name: "node1".to_string(),
            image: RecordId::new("node_image", "img"),
            index,
            lab: RecordId::new("lab", "lab"),
            mgmt_ipv4: mgmt_ipv4.map(str::to_string),
            mgmt_ipv6: mgmt_ipv6.map(str::to_string),
            mgmt_mac: None,
            state: NodeState::Running,
            cpu_count: None,
            memory: None,
            vnc_port: None,
            ssh_host_key: None,
            cpu_pinning: None,
            qemu_thread_cpus: None,
        }
    }

    #[test]
    fn test_saved_addresses_survive_a_layout_change() {
        // A node deployed under the old layout (node 1 at .11) keeps its address.
        let node = db_node(1, Some("172.31.1.11"), Some("fd00:b00b:0:1::b"));
        let (ipv4, ipv6) = node_management_addresses(&node, &mgmt_net(true)).unwrap();
        assert_eq!(ipv4.to_string(), "172.31.1.11");
        assert_eq!(ipv6.unwrap().to_string(), "fd00:b00b:0:1::b");
    }

    #[test]
    fn test_unsaved_addresses_come_from_the_node_block() {
        let node = db_node(1, None, None);
        let (ipv4, ipv6) = node_management_addresses(&node, &mgmt_net(true)).unwrap();
        assert_eq!(ipv4.to_string(), "172.31.1.21");
        assert_eq!(ipv6.unwrap().to_string(), "fd00:b00b:0:1::15");
    }

    #[test]
    fn test_no_ipv6_address_without_an_ipv6_network() {
        let node = db_node(1, Some("172.31.1.21"), Some("fd00:b00b:0:1::15"));
        let (_, ipv6) = node_management_addresses(&node, &mgmt_net(false)).unwrap();
        assert!(ipv6.is_none());
    }

    #[test]
    fn test_invalid_saved_address_is_an_error() {
        let node = db_node(1, Some("not-an-ip"), None);
        assert!(node_management_addresses(&node, &mgmt_net(false)).is_err());
    }

    fn bridge(index: u16, members: &[(&str, &str)]) -> DbBridge {
        DbBridge {
            id: None,
            index,
            bridge_name: format!("brs{index}-lab"),
            network_name: format!("sherpa-bridge{index}-lab"),
            lab: RecordId::new("lab", "lab"),
            members: members
                .iter()
                .map(|(node, interface)| data::BridgeMember {
                    node: RecordId::new("node", *node),
                    interface: interface.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn test_only_member_interfaces_map_to_their_bridge() {
        // Redeploy used to put every unlinked interface of a bridge's node on that bridge.
        let bridges = vec![
            bridge(0, &[("dev01", "eth1"), ("fr01", "eth1")]),
            bridge(1, &[("dev01", "eth3"), ("fr02", "eth1")]),
        ];
        let interfaces = node_bridge_interfaces(&bridges, &RecordId::new("node", "dev01"));
        assert_eq!(
            interfaces,
            HashMap::from([
                ("eth1".to_string(), "brs0-lab".to_string()),
                ("eth3".to_string(), "brs1-lab".to_string()),
            ])
        );
    }

    #[test]
    fn test_node_in_no_bridge_has_no_bridge_interfaces() {
        let bridges = vec![bridge(0, &[("fr01", "eth1")])];
        assert!(node_bridge_interfaces(&bridges, &RecordId::new("node", "dev01")).is_empty());
    }
}
