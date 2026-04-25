// Server-side implementation of the redeploy operation for a single node

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use opentelemetry::KeyValue;

use tracing::instrument;

use crate::daemon::state::AppState;
use crate::services::node_ops;
use crate::services::progress::ProgressSender;

use shared::data;
use shared::data::{NodeState, RedeployRequest, RedeployResponse, StatusKind};
use shared::konst::{
    BRIDGE_PREFIX, CONTAINER_VETH_PREFIX, KVM_OUI, LAB_FILE_NAME, READINESS_SLEEP,
    READINESS_TIMEOUT, SHERPA_LABS_PATH, SHERPA_MANAGEMENT_NETWORK_NAME, SSH_PORT, TAP_PREFIX,
    TFTP_DIR, ZTP_DIR,
};
use shared::util;

/// Tracks a veth pair created for a P2p container interface during redeploy.
struct P2pContainerVeth {
    host_veth: String,
    container_veth: String,
    interface_idx: u8,
    node_model: data::NodeModel,
    admin_down: bool,
}

/// Redeploy a single node: destroy existing + recreate with fresh ZTP
#[instrument(skip(state, progress), fields(lab_id = %request.lab_id, node_name = %request.node_name))]
pub async fn redeploy_node(
    request: RedeployRequest,
    state: &AppState,
    progress: ProgressSender,
) -> Result<RedeployResponse> {
    let start_time = Instant::now();
    let lab_id = &request.lab_id;
    let node_name = &request.node_name;

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

    // Find the target node in the manifest
    let manifest_nodes: Vec<topology::NodeExpanded> = manifest
        .nodes
        .iter()
        .enumerate()
        .map(|(idx, node)| topology::NodeExpanded {
            name: node.name.clone(),
            model: node.model,
            index: idx as u16 + 1,
            version: node.version.clone(),
            memory: node.memory,
            cpu_count: node.cpu_count,
            boot_disk_size: node.boot_disk_size,
            data_interface_count: node.data_interface_count,
            ipv4_address: node.ipv4_address,
            ipv6_address: node.ipv6_address,
            ssh_authorized_keys: node.ssh_authorized_keys.clone(),
            ssh_authorized_key_files: node.ssh_authorized_key_files.clone(),
            text_files: node.text_files_data.clone(),
            binary_files: node.binary_files.clone(),
            systemd_units: node.systemd_units.clone(),
            image: node.image.clone(),
            privileged: node.privileged,
            shm_size: node.shm_size,
            environment_variables: node.environment_variables.clone(),
            volumes: node.volumes.clone(),
            commands: node.commands.clone(),
            user: node.user.clone(),
            skip_ready_check: node.skip_ready_check,
            ztp_config: node.ztp_config.clone(),
            startup_scripts: node.startup_scripts_data.clone(),
            user_scripts: node.user_scripts_data.clone(),
            kernel_cmdline: node.kernel_cmdline.clone(),
            ready_port: node.ready_port,
        })
        .collect();

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

    // Use the shared database connection from AppState
    let db = state.db.clone();

    // Get lab info
    let db_lab = db::get_lab(&db, lab_id)
        .await
        .context(format!("Lab '{}' not found in database", lab_id))?;

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
    let data_interface_count = validate::effective_data_interface_count(
        node_name,
        target_node.data_interface_count,
        &node_image,
    )
    .context(format!(
        "Data interface count validation failed for node: {}",
        node_name
    ))?;

    // Load lab info from filesystem
    let lab_dir = format!("{SHERPA_LABS_PATH}/{lab_id}");
    let lab_file = util::load_file(&format!("{lab_dir}/{LAB_FILE_NAME}"))
        .context("Unable to load lab file")?;
    let lab_info: data::LabInfo = lab_file.parse().context("Failed to parse lab info file")?;

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
    let sherpa_user = util::sherpa_user().context("Failed to get sherpa user")?;
    let node_idx = db_node.index;
    let node_ip_idx = 10 + node_idx as u32;
    let node_ipv4_address = util::get_ipv4_addr(&mgmt_net.v4.prefix, node_ip_idx)?;

    // Assign IPv6 management address
    if let Some(ref v6) = mgmt_net.v6 {
        let addr = util::get_ipv6_addr(&v6.prefix, node_ip_idx)?;
        target_node.ipv6_address = Some(addr);
    }

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

    match node_image.kind {
        data::NodeKind::Container => {
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

            // Check if this node has any P2p links
            let is_p2p_container = db_links.iter().any(|link| {
                link.kind == data::BridgeKind::P2p
                    && (link.node_a == node_record_id || link.node_b == node_record_id)
            });

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
                // P2p container: create veth pairs for ALL data interfaces (linked + disabled).
                // Docker only handles the management network.

                // Create veths for P2p linked interfaces
                for link in &db_links {
                    if link.kind != data::BridgeKind::P2p {
                        continue;
                    }
                    if link.node_a == node_record_id {
                        let container_veth =
                            format!("{}a{}-{}", CONTAINER_VETH_PREFIX, link.index, lab_id);
                        network::create_veth_pair(
                            &link.tap_a,
                            &container_veth,
                            &format!("{}-p2p-host-{}::{}", lab_id, node_name, link.int_a),
                            &format!("{}-p2p-ctr-{}::{}", lab_id, node_name, link.int_a),
                        )
                        .await?;
                        let iface_idx = util::interface_to_idx(&target_node.model, &link.int_a)?;
                        p2p_container_veths.push(P2pContainerVeth {
                            host_veth: link.tap_a.clone(),
                            container_veth,
                            interface_idx: iface_idx,
                            node_model: target_node.model,
                            admin_down: false,
                        });
                    }
                    if link.node_b == node_record_id {
                        let container_veth =
                            format!("{}b{}-{}", CONTAINER_VETH_PREFIX, link.index, lab_id);
                        network::create_veth_pair(
                            &link.tap_b,
                            &container_veth,
                            &format!("{}-p2p-host-{}::{}", lab_id, node_name, link.int_b),
                            &format!("{}-p2p-ctr-{}::{}", lab_id, node_name, link.int_b),
                        )
                        .await?;
                        let iface_idx = util::interface_to_idx(&target_node.model, &link.int_b)?;
                        p2p_container_veths.push(P2pContainerVeth {
                            host_veth: link.tap_b.clone(),
                            container_veth,
                            interface_idx: iface_idx,
                            node_model: target_node.model,
                            admin_down: false,
                        });
                    }
                }

                // Create veths for disabled interfaces
                let linked_iface_idxs: std::collections::HashSet<u8> = p2p_container_veths
                    .iter()
                    .map(|v| v.interface_idx)
                    .collect();
                for idx in first_data_idx..=max_iface_idx {
                    if linked_iface_idxs.contains(&idx) {
                        continue;
                    }
                    let host_veth = format!("cd{}i{}-{}", node_idx, idx, lab_id);
                    let container_veth = format!("ce{}i{}-{}", node_idx, idx, lab_id);
                    network::create_veth_pair(
                        &host_veth,
                        &container_veth,
                        &format!("{}-p2p-disabled-host-{}::{}", lab_id, node_name, idx),
                        &format!("{}-p2p-disabled-ctr-{}::{}", lab_id, node_name, idx),
                    )
                    .await?;
                    p2p_container_veths.push(P2pContainerVeth {
                        host_veth,
                        container_veth,
                        interface_idx: idx,
                        node_model: target_node.model,
                        admin_down: true,
                    });
                }
            } else {
                // Non-P2p container: use Docker macvlan networks (existing behavior)
                let mut iface_net_lookup: std::collections::HashMap<String, (String, String)> =
                    std::collections::HashMap::new();

                for link in &db_links {
                    if link.node_a == node_record_id {
                        let docker_net_name =
                            format!("{}-etha{}-{}", node_name, link.index, lab_id);
                        iface_net_lookup
                            .insert(link.int_a.clone(), (docker_net_name, link.bridge_a.clone()));
                    }
                    if link.node_b == node_record_id {
                        let docker_net_name =
                            format!("{}-ethb{}-{}", node_name, link.index, lab_id);
                        iface_net_lookup
                            .insert(link.int_b.clone(), (docker_net_name, link.bridge_b.clone()));
                    }
                }

                let isolated = node_ops::node_isolated_network_data(node_name, node_idx, lab_id);

                for idx in first_data_idx..=max_iface_idx {
                    let iface_name = util::interface_from_idx(&target_node.model, idx)?;

                    if let Some((docker_net_name, bridge_name)) = iface_net_lookup.get(&iface_name)
                    {
                        container::create_docker_macvlan_network(
                            &docker_conn,
                            bridge_name,
                            docker_net_name,
                        )
                        .await?;

                        let linux_interface_name =
                            if target_node.model == data::NodeModel::NokiaSrlinux {
                                util::srlinux_to_linux_interface(&iface_name).ok()
                            } else {
                                None
                            };
                        additional_networks.push(data::ContainerNetworkAttachment {
                            name: docker_net_name.clone(),
                            ipv4_address: None,
                            ipv6_address: None,
                            linux_interface_name,
                            admin_down: false,
                        });
                    } else {
                        let docker_net_name = format!("{}-iso{}-{}", node_name, idx, lab_id);
                        container::create_docker_macvlan_bridge_network(
                            &docker_conn,
                            &isolated.bridge_name,
                            &docker_net_name,
                        )
                        .await?;

                        let linux_interface_name =
                            util::interface_from_idx(&target_node.model, idx)
                                .ok()
                                .and_then(|name| {
                                    if target_node.model == data::NodeModel::NokiaSrlinux {
                                        util::srlinux_to_linux_interface(&name).ok()
                                    } else {
                                        Some(name)
                                    }
                                });
                        additional_networks.push(data::ContainerNetworkAttachment {
                            name: docker_net_name,
                            ipv4_address: None,
                            ipv6_address: None,
                            linux_interface_name,
                            admin_down: true,
                        });
                    }
                }
            }

            let mgmt_attachment = data::ContainerNetworkAttachment {
                name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
                ipv4_address: Some(node_ipv4_address.to_string()),
                ipv6_address: None,
                linux_interface_name: None,
                admin_down: false,
            };

            let is_running = node_ops::start_container_node(
                &docker_conn,
                &container_name,
                &container_image,
                ztp_result.env_vars,
                ztp_result.volumes,
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

            // P2p post-start: move veths into container netns and attach eBPF
            if is_p2p_container && !p2p_container_veths.is_empty() {
                let pid = container::get_container_pid(&docker_conn, &container_name).await?;

                for veth_info in &p2p_container_veths {
                    network::move_to_netns(&veth_info.container_veth, pid).await?;

                    let target_name = if veth_info.node_model == data::NodeModel::NokiaSrlinux {
                        let iface_name = util::interface_from_idx(
                            &veth_info.node_model,
                            veth_info.interface_idx,
                        )?;
                        util::srlinux_to_linux_interface(&iface_name)?
                    } else {
                        format!("eth{}", veth_info.interface_idx)
                    };

                    let setup_cmd = if veth_info.admin_down {
                        format!(
                            "ip link set {} name {} && ip link set {} down",
                            veth_info.container_veth, target_name, target_name
                        )
                    } else {
                        format!(
                            "ip link set {} name {} && ip link set {} promisc on && ip link set {} up",
                            veth_info.container_veth, target_name, target_name, target_name
                        )
                    };
                    container::exec_container_with_retry(
                        &docker_conn,
                        &container_name,
                        vec!["sh", "-c", &setup_cmd],
                        3,
                        Duration::from_secs(2),
                    )
                    .await
                    .with_context(|| {
                        format!(
                            "Failed to configure P2p interface {} in container {}",
                            target_name, container_name
                        )
                    })?;

                    if veth_info.admin_down {
                        network::set_link_down(&veth_info.host_veth).await?;
                    }

                    tracing::info!(
                        lab_id = %lab_id,
                        container = %container_name,
                        veth = %veth_info.container_veth,
                        target = %target_name,
                        admin_down = veth_info.admin_down,
                        "Moved P2p veth into container netns (redeploy)"
                    );
                }

                // Re-attach eBPF redirect on P2p links involving this node
                for link in &db_links {
                    if link.kind != data::BridgeKind::P2p {
                        continue;
                    }
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

                    // Re-apply link impairment if configured
                    if link.delay_us > 0 || link.loss_percent > 0.0 {
                        let netem = network::LinkImpairment {
                            delay_us: link.delay_us,
                            jitter_us: link.jitter_us,
                            loss_percent: link.loss_percent,
                            reorder_percent: 0.0,
                            corrupt_percent: 0.0,
                        };
                        network::apply_netem(ifindex_a as i32, &netem).await?;
                        network::apply_netem(ifindex_b as i32, &netem).await?;
                    }

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

            node_ops::clone_node_disks(qemu_conn.clone(), vm_ztp.clone_disks, lab_id, &progress)
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
                interface_connection: None,
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
                    interface_connection: None,
                });
            }

            // Data interfaces
            let max_interface_idx = node_image
                .reserved_interface_count
                .checked_add(data_interface_count)
                .ok_or_else(|| anyhow!("Data interface count overflow for node {}", node_name))?;
            for idx in first_data_interface_idx..=max_interface_idx {
                let interface_name = util::interface_from_idx(&target_node.model, idx)?;

                // Check if this interface has a link
                let mut found_link = false;
                for link in &db_links {
                    if link.node_a == node_record_id && link.int_a == interface_name {
                        let is_p2p = link.kind == data::BridgeKind::P2p;
                        let iface_name = if is_p2p {
                            format!("{}a{}-{}", TAP_PREFIX, link.index, lab_id)
                        } else {
                            format!("{}a{}-{}", BRIDGE_PREFIX, link.index, lab_id)
                        };
                        let conn_type = if is_p2p {
                            data::ConnectionTypes::P2p
                        } else {
                            data::ConnectionTypes::PeerBridge
                        };
                        let source_node = db::get_node_by_id(&db, link.node_b.clone()).await?;
                        let interface_connection = data::InterfaceConnection {
                            local_id: node_idx,
                            local_port: util::id_to_port(node_idx as u8),
                            local_loopback: util::get_ip(&loopback_subnet, node_idx as u8)
                                .to_string(),
                            source_id: source_node.index,
                            source_port: util::id_to_port(source_node.index as u8),
                            source_loopback: util::get_ip(
                                &loopback_subnet,
                                source_node.index as u8,
                            )
                            .to_string(),
                        };
                        interfaces.push(data::Interface {
                            name: iface_name,
                            num: idx,
                            mtu: node_image.interface_mtu,
                            mac_address: util::random_mac(KVM_OUI),
                            connection_type: conn_type,
                            interface_connection: Some(interface_connection),
                        });
                        found_link = true;
                        break;
                    }
                    if link.node_b == node_record_id && link.int_b == interface_name {
                        let is_p2p = link.kind == data::BridgeKind::P2p;
                        let iface_name = if is_p2p {
                            format!("{}b{}-{}", TAP_PREFIX, link.index, lab_id)
                        } else {
                            format!("{}b{}-{}", BRIDGE_PREFIX, link.index, lab_id)
                        };
                        let conn_type = if is_p2p {
                            data::ConnectionTypes::P2p
                        } else {
                            data::ConnectionTypes::PeerBridge
                        };
                        let source_node = db::get_node_by_id(&db, link.node_a.clone()).await?;
                        let interface_connection = data::InterfaceConnection {
                            local_id: node_idx,
                            local_port: util::id_to_port(node_idx as u8),
                            local_loopback: util::get_ip(&loopback_subnet, node_idx as u8)
                                .to_string(),
                            source_id: source_node.index,
                            source_port: util::id_to_port(source_node.index as u8),
                            source_loopback: util::get_ip(
                                &loopback_subnet,
                                source_node.index as u8,
                            )
                            .to_string(),
                        };
                        interfaces.push(data::Interface {
                            name: iface_name,
                            num: idx,
                            mtu: node_image.interface_mtu,
                            mac_address: util::random_mac(KVM_OUI),
                            connection_type: conn_type,
                            interface_connection: Some(interface_connection),
                        });
                        found_link = true;
                        break;
                    }
                }

                // Check bridges
                if !found_link {
                    let db_bridges = db::list_bridges(&db, &lab_record_id).await?;
                    let mut found_bridge = false;
                    for bridge in &db_bridges {
                        if bridge.nodes.contains(&node_record_id) {
                            // Check if this node's interface connects to this bridge
                            // For now, treat as bridge interface
                            interfaces.push(data::Interface {
                                name: bridge.bridge_name.clone(),
                                num: idx,
                                mtu: node_image.interface_mtu,
                                mac_address: util::random_mac(KVM_OUI),
                                connection_type: data::ConnectionTypes::PrivateBridge,
                                interface_connection: None,
                            });
                            found_bridge = true;
                            break;
                        }
                    }

                    if !found_bridge {
                        // Disabled interface - connected to isolated network
                        interfaces.push(data::Interface {
                            name: util::dasher(&util::interface_from_idx(&target_node.model, idx)?),
                            num: idx,
                            mtu: node_image.interface_mtu,
                            mac_address: util::random_mac(KVM_OUI),
                            connection_type: data::ConnectionTypes::Disabled,
                            interface_connection: None,
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
            );

            let _ =
                progress.send_status(format!("Creating VM: {}", node_name), StatusKind::Progress);

            node_ops::create_vm(qemu_conn.clone(), domain, &progress).await?;

            // Re-attach eBPF redirect on P2p links involving this VM
            let p2p_vm_links: Vec<_> = db_links
                .iter()
                .filter(|l| {
                    l.kind == data::BridgeKind::P2p
                        && (l.node_a == node_record_id || l.node_b == node_record_id)
                })
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

                    if link.delay_us > 0 || link.loss_percent > 0.0 {
                        let netem = network::LinkImpairment {
                            delay_us: link.delay_us,
                            jitter_us: link.jitter_us,
                            loss_percent: link.loss_percent,
                            reorder_percent: 0.0,
                            corrupt_percent: 0.0,
                        };
                        network::apply_netem(ifindex_a as i32, &netem).await?;
                        network::apply_netem(ifindex_b as i32, &netem).await?;
                    }

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

            while !is_ready && ready_start.elapsed() < ready_timeout {
                match node_ops::check_node_ready_ssh(&ip_str, SSH_PORT)? {
                    true => {
                        is_ready = true;
                        let _ = progress.send_status(
                            format!("Node {} - Ready (SSH accessible)", node_name),
                            StatusKind::Done,
                        );
                    }
                    false => {
                        let _ = progress.send_status(
                            format!("Node {} - Waiting for SSH", node_name),
                            StatusKind::Waiting,
                        );
                        tokio::time::sleep(Duration::from_secs(READINESS_SLEEP)).await;
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
