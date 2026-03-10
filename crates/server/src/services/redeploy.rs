// Server-side implementation of the redeploy operation for a single node

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

use crate::daemon::state::AppState;
use crate::services::node_ops;
use crate::services::progress::ProgressSender;

use shared::data;
use shared::data::{NodeState, RedeployRequest, RedeployResponse, StatusKind};
use shared::konst::{
    BRIDGE_PREFIX, KVM_OUI, LAB_FILE_NAME, READINESS_SLEEP, READINESS_TIMEOUT, SHERPA_LABS_PATH,
    SHERPA_MANAGEMENT_NETWORK_NAME, SSH_PORT, TFTP_DIR, ZTP_DIR,
};
use shared::util;

/// Redeploy a single node: destroy existing + recreate with fresh ZTP
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
            ipv4_address: node.ipv4_address,
            ssh_authorized_keys: node.ssh_authorized_keys.clone(),
            ssh_authorized_key_files: node.ssh_authorized_key_files.clone(),
            text_files: node.text_files.clone(),
            binary_files: node.binary_files.clone(),
            systemd_units: node.systemd_units.clone(),
            image: node.image.clone(),
            privileged: node.privileged,
            environment_variables: node.environment_variables.clone(),
            volumes: node.volumes.clone(),
            commands: node.commands.clone(),
            user: node.user.clone(),
            skip_ready_check: node.skip_ready_check,
            ztp_config: node.ztp_config.clone(),
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

    // Connect to database
    let db = node_ops::connect_db().await?;

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
    };
    let dns = util::default_dns(&lab_info.ipv4_network)?;
    let sherpa_user = util::sherpa_user().context("Failed to get sherpa user")?;
    let node_idx = db_node.index;
    let node_ip_idx = 10 + node_idx as u32;
    let node_ipv4_address = util::get_ipv4_addr(&mgmt_net.v4.prefix, node_ip_idx)?;

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
            let mut iface_net_lookup: std::collections::HashMap<String, (String, String)> =
                std::collections::HashMap::new();

            for link in &db_links {
                if link.node_a == node_record_id {
                    let docker_net_name = format!("{}-etha{}-{}", node_name, link.index, lab_id);
                    iface_net_lookup
                        .insert(link.int_a.clone(), (docker_net_name, link.bridge_a.clone()));
                }
                if link.node_b == node_record_id {
                    let docker_net_name = format!("{}-ethb{}-{}", node_name, link.index, lab_id);
                    iface_net_lookup
                        .insert(link.int_b.clone(), (docker_net_name, link.bridge_b.clone()));
                }
            }

            // Determine isolated network bridge for disabled interfaces
            let first_data_idx = 1 + node_image.reserved_interface_count;
            let max_iface_idx = first_data_idx + node_image.data_interface_count - 1;
            let isolated = node_ops::node_isolated_network_data(node_name, node_idx, lab_id);

            // Walk interfaces in model-index order and build attachments
            let mut additional_networks = vec![];
            for idx in 0..=max_iface_idx {
                // Skip management (idx 0) and reserved interfaces
                if idx < first_data_idx {
                    continue;
                }

                let iface_name = util::interface_from_idx(&target_node.model, idx)?;

                if let Some((docker_net_name, bridge_name)) = iface_net_lookup.get(&iface_name) {
                    // Linked interface — recreate Docker macvlan network
                    container::create_docker_macvlan_network(
                        &docker_conn,
                        bridge_name,
                        docker_net_name,
                    )
                    .await?;

                    let linux_interface_name = if target_node.model == data::NodeModel::NokiaSrlinux
                    {
                        util::srlinux_to_linux_interface(&iface_name).ok()
                    } else {
                        None
                    };
                    additional_networks.push(data::ContainerNetworkAttachment {
                        name: docker_net_name.clone(),
                        ipv4_address: None,
                        linux_interface_name,
                        admin_down: false,
                    });
                } else {
                    // Disabled interface — recreate isolated Docker macvlan bridge-mode network
                    let docker_net_name = format!("{}-iso{}-{}", node_name, idx, lab_id);
                    container::create_docker_macvlan_bridge_network(
                        &docker_conn,
                        &isolated.bridge_name,
                        &docker_net_name,
                    )
                    .await?;

                    let linux_interface_name = util::interface_from_idx(&target_node.model, idx)
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
                        linux_interface_name,
                        admin_down: true,
                    });
                }
            }

            let mgmt_attachment = data::ContainerNetworkAttachment {
                name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
                ipv4_address: Some(node_ipv4_address.to_string()),
                linux_interface_name: None,
                admin_down: false,
            };

            let is_running = node_ops::start_container_node(
                &docker_conn,
                &container_name,
                &container_image,
                ztp_result.env_vars,
                ztp_result.volumes,
                mgmt_attachment,
                additional_networks,
                ztp_result.commands,
                ztp_result.privileged,
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
            let first_data_interface_idx = 1 + node_image.reserved_interface_count;
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
            let max_interface_idx = first_data_interface_idx + node_image.data_interface_count - 1;
            for idx in first_data_interface_idx..=max_interface_idx {
                let interface_name = util::interface_from_idx(&target_node.model, idx)?;

                // Check if this interface has a link
                let mut found_link = false;
                for link in &db_links {
                    if link.node_a == node_record_id && link.int_a == interface_name {
                        let bridge_name = format!("{}a{}-{}", BRIDGE_PREFIX, link.index, lab_id);
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
                            name: bridge_name,
                            num: idx,
                            mtu: node_image.interface_mtu,
                            mac_address: util::random_mac(KVM_OUI),
                            connection_type: data::ConnectionTypes::PeerBridge,
                            interface_connection: Some(interface_connection),
                        });
                        found_link = true;
                        break;
                    }
                    if link.node_b == node_record_id && link.int_b == interface_name {
                        let bridge_name = format!("{}b{}-{}", BRIDGE_PREFIX, link.index, lab_id);
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
                            name: bridge_name,
                            num: idx,
                            mtu: node_image.interface_mtu,
                            mac_address: util::random_mac(KVM_OUI),
                            connection_type: data::ConnectionTypes::PeerBridge,
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
