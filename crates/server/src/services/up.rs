// Server-side implementation of the lab startup operation
// This is a port of the client's up.rs command with streaming progress support

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use askama::Template;
use serde_json;

use crate::daemon::state::AppState;
use crate::services::clean;
use crate::services::node_ops;
use crate::services::progress::ProgressSender;
use crate::tls;

use shared::data;
use shared::data::{NodeState, StatusKind};
use shared::konst::{
    BRIDGE_PREFIX, CONTAINER_DNSMASQ_NAME, CONTAINER_DNSMASQ_REPO, DNSMASQ_CONFIG_FILE,
    DNSMASQ_DIR, DNSMASQ_LEASES_FILE, KVM_OUI, LAB_CA_CERT_FILE, LAB_CA_KEY_FILE,
    LAB_CERT_VALIDITY_DAYS, LAB_CERTS_DIR, LAB_FILE_NAME, NODE_CONFIGS_DIR, READINESS_SLEEP,
    READINESS_TIMEOUT, SHERPA_CONFIG_FILE_PATH, SHERPA_LABS_PATH, SHERPA_LOOPBACK_PREFIX,
    SHERPA_LOOPBACK_PREFIX_IPV6, SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX,
    SHERPA_MANAGEMENT_NETWORK_IPV6, SHERPA_MANAGEMENT_NETWORK_NAME, SHERPA_SSH_CONFIG_FILE,
    SHERPA_SSH_PRIVATE_KEY_PATH, SSH_PORT, TFTP_DIR, VETH_PREFIX, ZTP_DIR,
};
use shared::util;

// ============================================================================
// ============================================================================
// Helper Functions
// ============================================================================
// node_isolated_network_data, node_reserved_network_data, ztp_config_filename,
// take_custom_ztp_config, get_node_data — imported from node_ops module

fn find_interface_link(
    node_name: &str,
    interface_name: &str,
    links_detailed: &Vec<topology::LinkDetailed>,
) -> Option<data::NodeInterface> {
    let mut interface_data = None;
    for link in links_detailed {
        if link.node_a == node_name && link.int_a == *interface_name {
            interface_data = Some(data::NodeInterface::Peer(data::PeerInterface {
                link_index: link.link_idx,
                this_node: link.node_a.clone(),
                this_node_index: link.node_a_idx,
                this_interface: link.int_a.clone(),
                this_interface_index: link.int_a_idx,
                this_side: data::PeerSide::A,
                peer_node: link.node_b.clone(),
                peer_node_index: link.node_b_idx,
                peer_interface: link.int_b.clone(),
                peer_interface_index: link.int_b_idx,
                peer_side: data::PeerSide::B,
            }))
        } else if link.node_b == node_name && link.int_b == *interface_name {
            interface_data = Some(data::NodeInterface::Peer(data::PeerInterface {
                link_index: link.link_idx,
                this_node: link.node_b.clone(),
                this_node_index: link.node_b_idx,
                this_interface: link.int_b.clone(),
                this_interface_index: link.int_b_idx,
                this_side: data::PeerSide::B,
                peer_node: link.node_a.clone(),
                peer_node_index: link.node_a_idx,
                peer_interface: link.int_a.clone(),
                peer_interface_index: link.int_a_idx,
                peer_side: data::PeerSide::A,
            }))
        }
    }
    interface_data
}

fn find_bridge_interface(
    node_name: &str,
    interface_name: &str,
    bridge_connections: &[topology::BridgeDetailed],
) -> Option<data::NodeInterface> {
    let mut interface_data = None;
    for bridge in bridge_connections.iter() {
        for link in bridge.links.iter() {
            if link.node_name == node_name && link.interface_name == *interface_name {
                interface_data = Some(data::NodeInterface::Bridge(data::BridgeInterface {
                    name: bridge.bridge_name.clone(),
                }))
            }
        }
    }
    interface_data
}

fn process_manifest_bridges(
    manifest_bridges: &Option<Vec<topology::Bridge>>,
    manifest_nodes: &[topology::NodeExpanded],
    lab_id: &str,
) -> Result<Vec<topology::BridgeDetailed>> {
    let manifest_bridges = manifest_bridges.clone().unwrap_or_default();
    let bridges = manifest_bridges
        .iter()
        .map(|x: &topology::Bridge| x.parse_links())
        .collect::<Result<Vec<_>>>()?;

    let mut bridges_detailed = vec![];
    for (bridge_idx, bridge) in bridges.iter().enumerate() {
        let mut bridge_links = vec![];

        for link in bridge.links.iter() {
            if let Some(node) = manifest_nodes.iter().find(|n| n.name == link.node) {
                let interface_idx = util::interface_to_idx(&node.model, &link.interface)?;
                bridge_links.push(topology::BridgeLinkDetailed {
                    node_name: link.node.clone(),
                    node_model: node.model,
                    interface_name: link.interface.clone(),
                    interface_index: interface_idx,
                });
            }
        }

        bridges_detailed.push(topology::BridgeDetailed {
            manifest_name: bridge.name.clone(),
            libvirt_name: format!("sherpa-bridge{}-{}-{}", bridge_idx, bridge.name, lab_id),
            bridge_name: format!("{}s{}-{}", BRIDGE_PREFIX, bridge_idx, lab_id),
            index: bridge_idx as u16,
            links: bridge_links,
        });
    }
    Ok(bridges_detailed)
}

/// Process manifest nodes into expanded format with indices assigned
fn process_manifest_nodes(manifest_nodes: &[topology::Node]) -> Vec<topology::NodeExpanded> {
    manifest_nodes
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
            ipv6_address: node.ipv6_address,
            ssh_authorized_keys: node.ssh_authorized_keys.clone(),
            ssh_authorized_key_files: node.ssh_authorized_key_files.clone(),
            text_files: node.text_files.clone(),
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
        })
        .collect()
}

/// Process manifest links into detailed link format with resolved interface indices
fn process_manifest_links(
    manifest_links: &Option<Vec<topology::Link2>>,
    manifest_nodes: &[topology::NodeExpanded],
) -> Result<Vec<topology::LinkDetailed>> {
    let manifest_links = manifest_links.clone().unwrap_or_default();
    let links = manifest_links
        .iter()
        .map(|x: &topology::Link2| x.expand())
        .collect::<Result<Vec<topology::LinkExpanded>>>()?;

    let mut links_detailed = vec![];
    for (link_idx, link) in links.iter().enumerate() {
        let mut this_link = topology::LinkDetailed::default();
        for device in manifest_nodes.iter() {
            let device_model = device.model;
            if link.node_a == device.name {
                let int_idx = util::interface_to_idx(&device_model, &link.int_a)?;
                let peer_node = manifest_nodes
                    .iter()
                    .find(|n| n.name == link.node_b)
                    .ok_or_else(|| anyhow!("Peer node not found: {}", link.node_b))?;
                this_link.node_a = device.name.clone();
                this_link.node_a_idx = device.index;
                this_link.node_a_model = device_model;
                this_link.int_a = link.int_a.clone();
                this_link.int_a_idx = int_idx;
                this_link.link_idx = link_idx as u16;
                this_link.node_b_idx = peer_node.index;
            } else if link.node_b == device.name {
                let peer_node = manifest_nodes
                    .iter()
                    .find(|n| n.name == link.node_a)
                    .ok_or_else(|| anyhow!("Peer node not found: {}", link.node_a))?;
                let int_idx = util::interface_to_idx(&device_model, &link.int_b)?;
                this_link.node_b = device.name.clone();
                this_link.node_b_idx = device.index;
                this_link.node_b_model = device_model;
                this_link.int_b = link.int_b.clone();
                this_link.int_b_idx = int_idx;
                this_link.link_idx = link_idx as u16;
                this_link.node_a_idx = peer_node.index;
            }
        }
        links_detailed.push(this_link)
    }
    Ok(links_detailed)
}

/// Get node image from a list of node images.
/// When a version is provided, match on that version. Otherwise fall back to the default.
fn get_node_image(
    node_model: &data::NodeModel,
    version: Option<&str>,
    data: &[data::NodeConfig],
) -> Result<data::NodeConfig> {
    if let Some(v) = version {
        data.iter()
            .find(|x| &x.model == node_model && x.version == v)
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "Node image not found for model: {} version: {}",
                    node_model,
                    v
                )
            })
    } else {
        data.iter()
            .find(|x| &x.model == node_model && x.default)
            .cloned()
            .ok_or_else(|| anyhow!("Default node image not found for model: {}", node_model))
    }
}

// ============================================================================
// Main Up Service Function
// ============================================================================

/// Start a lab with streaming progress updates
pub async fn up_lab(
    request: data::UpRequest,
    state: &AppState,
    progress: ProgressSender,
) -> Result<data::UpResponse> {
    // TODO: Currently accepts username without authentication. This assumes a trusted
    // environment where the client can be trusted to send correct username. In production,
    // this should be replaced with proper authentication (JWT, session, etc.) where the
    // username is extracted from a verified token rather than client-provided param.

    let start_time = Instant::now();
    let mut phases_completed = Vec::new();
    let mut errors = Vec::new();

    let lab_id = &request.lab_id;

    // Deserialize manifest from JSON Value
    let manifest: topology::Manifest =
        serde_json::from_value(request.manifest).context("Failed to deserialize manifest")?;

    tracing::info!(
        "Starting lab creation: lab_id={}, name={}",
        lab_id,
        manifest.name
    );

    // ========================================================================
    // PHASE 1: Setup & Connections
    // ========================================================================
    let _ = progress.send_phase(data::UpPhase::Setup, "Initializing connections".to_string());

    tracing::info!(lab_id = %lab_id, lab_name = %manifest.name, "Connecting to lab infrastructure services");

    let sherpa_user = util::sherpa_user().context("Failed to get sherpa user")?;
    let lab_dir = format!("{SHERPA_LABS_PATH}/{lab_id}");
    let current_user = &request.username;
    let management_network = format!("{}-{}", SHERPA_MANAGEMENT_NETWORK_NAME, lab_id);

    // Get connections from AppState
    let docker_conn = state.docker.clone();
    tracing::info!(lab_id = %lab_id, "Connected to Docker daemon");

    let qemu_conn = Arc::new(
        state
            .qemu
            .connect()
            .context("Failed to connect to libvirt")?,
    );
    tracing::info!(lab_id = %lab_id, "Connected to libvirt/QEMU");

    // Connect to database
    let db = node_ops::connect_db().await?;
    tracing::info!(lab_id = %lab_id, "Connected to SurrealDB database");

    tracing::debug!(lab_id = %lab_id, lab_dir = %lab_dir, user = %current_user, "Lab environment prepared");

    let db_user = db::get_user(&db, current_user)
        .await
        .context("Failed to get database user")?;

    // Check if lab already exists (CRITICAL ERROR - fail fast)
    if let Ok(lab) = db::get_lab(&db, lab_id).await {
        bail!(
            "Lab already exists. Please use a different lab ID or destroy the existing lab first.\n Lab name: {}\n Lab id: {}",
            lab.name,
            lab_id,
        );
    }

    let _ = progress.send_status("Loading configuration".to_string(), StatusKind::Info);
    let config = state.config.clone();

    // Bulk fetch all node configs from database
    let node_images = db::list_node_images(&db)
        .await
        .context("Failed to list node configs from database")?;

    phases_completed.push("Setup".to_string());

    // ========================================================================
    // PHASE 2: Manifest Validation
    // ========================================================================
    let _ = progress.send_phase(
        data::UpPhase::ManifestValidation,
        "Validating manifest structure".to_string(),
    );

    tracing::info!(
        lab_id = %lab_id,
        lab_name = %manifest.name,
        "Validating lab manifest"
    );

    tracing::debug!(lab_id = %lab_id, node_images = node_images.len(), "Fetched node configs from database");

    // Device Validators (CRITICAL ERROR - fail fast on validation failure)
    validate::check_duplicate_device(&manifest.nodes)
        .context("Manifest validation failed: duplicate devices")?;

    // Environment variable validators
    for node in &manifest.nodes {
        if let Some(ref env_vars) = node.environment_variables {
            validate::validate_environment_variables(env_vars, &node.name)
                .context("Manifest validation failed: environment variables")?;
        }
    }

    // Version & Image Validators (CRITICAL ERROR - fail fast on validation failure)
    // Fetch local Docker images for validation
    let docker_images = container::get_local_images(&docker_conn)
        .await
        .context("Failed to list local Docker images")?;

    let validated_nodes = validate::validate_and_resolve_node_versions(
        &manifest.nodes,
        &node_images,
        &config.images_dir,
        &docker_images,
    )
    .context("Manifest validation failed: version/image validation")?;

    let nodes_expanded = process_manifest_nodes(&validated_nodes);
    let links_detailed = process_manifest_links(&manifest.links, &nodes_expanded)
        .context("Failed to process manifest links")?;
    let bridges_detailed = process_manifest_bridges(&manifest.bridges, &nodes_expanded, lab_id)
        .context("Failed to process manifest bridges")?;

    tracing::info!(
        lab_id = %lab_id,
        nodes = nodes_expanded.len(),
        links = links_detailed.len(),
        bridges = bridges_detailed.len(),
        "Processed manifest structures"
    );

    let mut ztp_records = vec![];

    for node in &nodes_expanded {
        let node_image = get_node_image(&node.model, node.version.as_deref(), &node_images)
            .context(format!("Node config not found for model: {}", node.model))?;

        if !node_image.dedicated_management_interface {
            validate::check_mgmt_usage(&node.name, 0, &links_detailed, &bridges_detailed).context(
                format!(
                    "Management interface validation failed for node: {}",
                    node.name
                ),
            )?;
        }

        validate::check_interface_bounds(
            &node.name,
            &node_image.model,
            node_image.data_interface_count,
            node_image.reserved_interface_count,
            node_image.dedicated_management_interface,
            &links_detailed,
            &bridges_detailed,
        )
        .context(format!(
            "Interface bounds validation failed for node: {}",
            node.name
        ))?;
    }

    // Connection Validators
    if !links_detailed.is_empty() {
        validate::check_duplicate_interface_link(&links_detailed, &bridges_detailed)
            .context("Duplicate interface link validation failed")?;
        validate::check_link_device(&manifest.nodes, &links_detailed)
            .context("Link device validation failed")?;
    }

    // Bridge Validators
    if !bridges_detailed.is_empty() {
        validate::check_bridge_device(&manifest.nodes, &bridges_detailed)
            .context("Bridge device validation failed")?;
    }

    let _ = progress.send_status("Manifest validation complete".to_string(), StatusKind::Done);
    tracing::info!(lab_id = %lab_id, "Manifest validation completed successfully");
    phases_completed.push("ManifestValidation".to_string());

    // Wrap resource-creating phases in a block so we can auto-clean on failure.
    // Phases 1-2 (Setup, ManifestValidation) don't create resources, so failures
    // there propagate directly without cleanup.
    let resource_creation = async {
        // ========================================================================
        // PHASE 3: Database Records & Data Structure Building
        // ========================================================================
        let _ = progress.send_phase(
            data::UpPhase::DatabaseRecords,
            "Creating database records".to_string(),
        );

        tracing::info!(lab_id = %lab_id, lab_name = %manifest.name, "Creating database records");

        // Allocate a unique loopback /24 subnet for this lab
        let loopback_prefix = util::get_ipv4_network(SHERPA_LOOPBACK_PREFIX)
            .context("Failed to parse loopback prefix")?;
        let used_loopback_networks = db::get_used_loopback_networks(&db)
            .await
            .context("Failed to query existing loopback networks")?;
        let loopback_subnet =
            util::allocate_loopback_subnet(&loopback_prefix, &used_loopback_networks)
                .context("Failed to allocate loopback subnet for lab")?;

        tracing::info!(
            lab_id = %lab_id,
            loopback_subnet = %loopback_subnet,
            "Allocated loopback subnet for lab"
        );

        // Allocate a unique management /24 subnet for this lab
        let management_prefix = config.management_prefix_ipv4;
        let used_management_networks = db::get_used_management_networks(&db)
            .await
            .context("Failed to query existing management networks")?;
        let management_subnet =
            util::allocate_management_subnet(&management_prefix, &used_management_networks)
                .context("Failed to allocate management subnet for lab")?;

        tracing::info!(
            lab_id = %lab_id,
            management_subnet = %management_subnet,
            "Allocated management subnet for lab"
        );

        // Allocate IPv6 management and loopback subnets
        let ipv6_mgmt_prefix = config.management_prefix_ipv6.unwrap_or_else(|| {
            util::get_ipv6_network(SHERPA_MANAGEMENT_NETWORK_IPV6)
                .expect("Failed to parse default IPv6 management prefix")
        });
        let ipv6_loop_prefix = util::get_ipv6_network(SHERPA_LOOPBACK_PREFIX_IPV6)
            .context("Failed to parse IPv6 loopback prefix")?;

        let used_ipv6_mgmt = db::get_used_ipv6_management_networks(&db)
            .await
            .context("Failed to query existing IPv6 management networks")?;
        let ipv6_management_subnet =
            util::allocate_ipv6_management_subnet(&ipv6_mgmt_prefix, &used_ipv6_mgmt)
                .context("Failed to allocate IPv6 management subnet for lab")?;

        let used_ipv6_loop = db::get_used_ipv6_loopback_networks(&db)
            .await
            .context("Failed to query existing IPv6 loopback networks")?;
        let ipv6_loopback_subnet =
            util::allocate_ipv6_loopback_subnet(&ipv6_loop_prefix, &used_ipv6_loop)
                .context("Failed to allocate IPv6 loopback subnet for lab")?;

        let gateway_ipv6 = util::get_ipv6_addr(&ipv6_management_subnet, 1)?;
        let router_ipv6 = util::get_ipv6_addr(&ipv6_management_subnet, 2)?;

        tracing::info!(
            lab_id = %lab_id,
            ipv6_management_subnet = %ipv6_management_subnet,
            ipv6_loopback_subnet = %ipv6_loopback_subnet,
            "Allocated IPv6 subnets for lab"
        );

        // Compute gateway and router IPs from management subnet
        let gateway_ipv4 = util::get_ipv4_addr(&management_subnet, 1)?;
        let router_ipv4 = util::get_ipv4_addr(&management_subnet, 2)?;

        // Create lab record in database
        let lab_record = db::create_lab(
            &db,
            &manifest.name,
            lab_id,
            &db_user,
            &loopback_subnet.to_string(),
            &management_subnet.to_string(),
            &gateway_ipv4.to_string(),
            &router_ipv4.to_string(),
        )
        .await
        .context("Failed to create lab record in database")?;

        // Update lab with IPv6 network data
        let mut lab_record = lab_record;
        lab_record.management_network_v6 = Some(ipv6_management_subnet.to_string());
        lab_record.gateway_ipv6 = Some(gateway_ipv6.to_string());
        lab_record.router_ipv6 = Some(router_ipv6.to_string());
        lab_record.loopback_network_v6 = Some(ipv6_loopback_subnet.to_string());
        let lab_record = db::update_lab(&db, lab_record)
            .await
            .context("Failed to update lab with IPv6 network data")?;

        let lab_record_id = db::get_lab_id(&lab_record).context("Failed to get lab record ID")?;

        tracing::info!(lab_id = %lab_id, lab_name = %manifest.name, "Created lab database record");

        let mut container_nodes: Vec<topology::NodeExpanded> = vec![];
        let mut unikernel_nodes: Vec<topology::NodeExpanded> = vec![];
        let mut vm_nodes: Vec<topology::NodeExpanded> = vec![];
        let mut clone_disks: Vec<data::CloneDisk> = vec![];
        let mut domains: Vec<template::DomainTemplate> = vec![];

        let mut lab_node_data = vec![];
        let mut node_setup_data = vec![];

        for node in nodes_expanded.iter() {
            let node_image = get_node_image(&node.model, node.version.as_deref(), &node_images)?;

            tracing::info!(
                lab_id = %lab_id,
                node_name = %node.name,
                node_kind = ?node_image.kind,
                node_model = ?node_image.model,
                "Creating node database record"
            );

            // Build interface data structures
            let mut node_interfaces_detailed: Vec<data::InterfaceData> = vec![];
            let first_data_interface_idx = 1 + node_image.reserved_interface_count;
            let max_interface_idx = first_data_interface_idx + node_image.data_interface_count - 1;

            for idx in 0..=max_interface_idx {
                let interface_name = util::interface_from_idx(&node.model, idx)?;
                let interface_idx = idx;
                let mut interface_state = data::InterfaceState::Enabled;
                let mut interface_data = data::NodeInterface::Disabled;

                if idx == 0 {
                    interface_data = data::NodeInterface::Management;
                } else if idx < first_data_interface_idx {
                    interface_data = data::NodeInterface::Reserved;
                } else {
                    if let Some(data) =
                        find_interface_link(&node.name, &interface_name, &links_detailed)
                    {
                        interface_data = data
                    }
                    if let Some(data) =
                        find_bridge_interface(&node.name, &interface_name, &bridges_detailed)
                    {
                        interface_data = data
                    }
                    interface_state = data::InterfaceState::Disabled;
                }

                node_interfaces_detailed.push(data::InterfaceData {
                    name: interface_name.to_string(),
                    index: interface_idx,
                    state: interface_state,
                    data: interface_data,
                });
            }

            let lab_node = db::create_node(
                &db,
                &node.name,
                node.index,
                db::get_image_id(&node_image)?,
                lab_record_id.clone(),
            )
            .await?;

            lab_node_data.push(data::LabNodeData {
                name: node.name.clone(),
                model: node_image.model,
                kind: node_image.kind.clone(),
                index: node.index,
                record: lab_node,
            });

            match node_image.kind {
                data::NodeKind::Container => {
                    container_nodes.push(node.clone());
                }
                data::NodeKind::Unikernel => {
                    unikernel_nodes.push(node.clone());
                }
                data::NodeKind::VirtualMachine => {
                    vm_nodes.push(node.clone());
                }
            }

            let has_disabled_interfaces = node_interfaces_detailed
                .iter()
                .any(|i| matches!(i.data, data::NodeInterface::Disabled));

            let node_isolated_network = if matches!(
                node_image.kind,
                data::NodeKind::VirtualMachine | data::NodeKind::Container
            ) && has_disabled_interfaces
            {
                Some(node_ops::node_isolated_network_data(
                    &node.name, node.index, lab_id,
                ))
            } else {
                None
            };

            let node_reserved_network = if matches!(node_image.kind, data::NodeKind::VirtualMachine)
                && node_image.reserved_interface_count > 0
            {
                Some(node_ops::node_reserved_network_data(
                    &node.name, node.index, lab_id,
                ))
            } else {
                None
            };

            if let Some(network) = node_isolated_network.clone() {
                let _ = progress.send_status(
                    format!("Creating isolated network for node: {}", node.name),
                    StatusKind::Progress,
                );
                tracing::info!(
                    lab_id = %lab_id,
                    node_name = %node.name,
                    network_type = "isolated",
                    "Creating node isolated network"
                );
                match node_image.kind {
                    data::NodeKind::VirtualMachine => {
                        let node_isolated_network = libvirt::IsolatedNetwork {
                            network_name: network.network_name,
                            bridge_name: network.bridge_name,
                        };
                        node_isolated_network.create(&qemu_conn)?;
                    }
                    data::NodeKind::Container => {
                        network::create_bridge(&network.bridge_name, &network.network_name).await?;
                    }
                    _ => {}
                }
            }

            if let Some(network) = node_reserved_network.clone() {
                let _ = progress.send_status(
                    format!("Creating reserved network for node: {}", node.name),
                    StatusKind::Progress,
                );
                tracing::info!(
                    lab_id = %lab_id,
                    node_name = %node.name,
                    network_type = "reserved",
                    "Creating node reserved network"
                );
                let node_reserved_network = libvirt::ReservedNetwork {
                    network_name: network.network_name,
                    bridge_name: network.bridge_name,
                };
                node_reserved_network.create(&qemu_conn)?;
            }

            node_setup_data.push(data::NodeSetupData {
                name: node.name.clone(),
                index: node.index,
                management_network: management_network.clone(),
                isolated_network: node_isolated_network,
                reserved_network: node_reserved_network,
                interfaces: node_interfaces_detailed,
            });
        }

        tracing::info!(
            lab_id = %lab_id,
            total_nodes = nodes_expanded.len(),
            containers = container_nodes.len(),
            vms = vm_nodes.len(),
            unikernels = unikernel_nodes.len(),
            "Created all node database records"
        );

        phases_completed.push("DatabaseRecords".to_string());

        // ========================================================================
        // PHASE 4: Lab Network Setup
        // ========================================================================
        let _ = progress.send_phase(
            data::UpPhase::LabNetworkSetup,
            "Allocating lab network and creating management network".to_string(),
        );

        let lab_net = management_subnet;

        tracing::info!(
            lab_id = %lab_id,
            subnet = %lab_net,
            gateway = %gateway_ipv4,
            boot_server = %router_ipv4,
            "Allocated lab network subnet"
        );

        let lab_info = data::LabInfo {
            id: lab_id.to_string(),
            user: current_user.clone(),
            name: manifest.name.clone(),
            ipv4_network: lab_net,
            ipv4_gateway: gateway_ipv4,
            ipv4_router: router_ipv4,
            loopback_network: loopback_subnet,
            ipv6_network: Some(ipv6_management_subnet),
            ipv6_gateway: Some(gateway_ipv6),
            ipv6_router: Some(router_ipv6),
        };

        util::create_dir(&lab_dir)?;
        util::create_file(&format!("{lab_dir}/{LAB_FILE_NAME}"), lab_info.to_string())?;

        let mgmt_net = data::SherpaNetwork {
            v4: data::NetworkV4 {
                prefix: lab_net,
                first: gateway_ipv4,
                last: lab_net.broadcast(),
                boot_server: router_ipv4,
                network: lab_net.network(),
                subnet_mask: lab_net.netmask(),
                hostmask: lab_net.hostmask(),
                prefix_length: lab_net.prefix_len(),
            },
            v6: Some(data::NetworkV6 {
                prefix: ipv6_management_subnet,
                first: gateway_ipv6,
                last: util::get_ipv6_addr(&ipv6_management_subnet, u32::MAX)?,
                boot_server: router_ipv6,
                network: ipv6_management_subnet.network(),
                prefix_length: ipv6_management_subnet.prefix_len(),
            }),
        };
        let dns = util::default_dns_dual_stack(&lab_net, &ipv6_management_subnet)?;

        let _ = progress.send_status(
            format!("Creating management network: {SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
            StatusKind::Progress,
        );

        tracing::info!(
            lab_id = %lab_id,
            network = %format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
            "Creating management network"
        );

        // Libvirt management network
        let management_network_obj = libvirt::NatNetwork {
            network_name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
            bridge_name: format!("{SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX}-{lab_id}"),
            ipv4_address: gateway_ipv4,
            ipv4_netmask: lab_net.netmask(),
            ipv6_address: Some(gateway_ipv6),
            ipv6_prefix_length: Some(ipv6_management_subnet.prefix_len()),
        };
        management_network_obj.create(&qemu_conn)?;

        tracing::info!(
            lab_id = %lab_id,
            network = %format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
            bridge = %format!("{SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX}-{lab_id}"),
            "Created libvirt NAT network"
        );

        // Docker management network
        container::create_docker_bridge_network(
            &docker_conn,
            &format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
            Some(lab_net.to_string()),
            Some(ipv6_management_subnet.to_string()),
            &format!("{SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX}-{lab_id}"),
        )
        .await?;

        tracing::info!(
            lab_id = %lab_id,
            network = %format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
            "Created Docker bridge network"
        );

        phases_completed.push("LabNetworkSetup".to_string());

        // ========================================================================
        // PHASE 5: Point-to-Point Link Creation
        // ========================================================================
        let _ = progress.send_phase(
            data::UpPhase::LinkCreation,
            format!("Creating {} point-to-point links", links_detailed.len()),
        );

        tracing::info!(
            lab_id = %lab_id,
            link_count = links_detailed.len(),
            "Creating point-to-point links"
        );

        let mut lab_link_data = vec![];

        for (idx, link) in links_detailed.iter().enumerate() {
            let node_a = lab_node_data
                .iter()
                .find(|n| n.name == link.node_a)
                .ok_or_else(|| anyhow!("Node not found: {}", link.node_a))?;

            let node_b = lab_node_data
                .iter()
                .find(|n| n.name == link.node_b)
                .ok_or_else(|| anyhow!("Node not found: {}", link.node_b))?;

            let bridge_a = format!("{}a{}-{}", BRIDGE_PREFIX, link.link_idx, lab_id);
            let bridge_b = format!("{}b{}-{}", BRIDGE_PREFIX, link.link_idx, lab_id);
            let veth_a = format!("{}a{}-{}", VETH_PREFIX, link.link_idx, lab_id);
            let veth_b = format!("{}b{}-{}", VETH_PREFIX, link.link_idx, lab_id);

            // Create the link in the database
            let _db_link = db::create_link(
                &db,
                link.link_idx,
                data::BridgeKind::P2pBridge,
                db::get_node_id(&node_a.record)?,
                db::get_node_id(&node_b.record)?,
                link.int_a.clone(),
                link.int_b.clone(),
                bridge_a.clone(),
                bridge_b.clone(),
                veth_a.clone(),
                veth_b.clone(),
                lab_record_id.clone(),
            )
            .await?;

            lab_link_data.push(data::LabLinkData {
                index: link.link_idx,
                kind: data::BridgeKind::P2pBridge,
                node_a: node_a.record.clone(),
                node_b: node_b.record.clone(),
                int_a: link.int_a.clone(),
                int_b: link.int_b.clone(),
                bridge_a: bridge_a.clone(),
                bridge_b: bridge_b.clone(),
                veth_a: veth_a.clone(),
                veth_b: veth_b.clone(),
            });

            let _ = progress.send_status(
                format!(
                    "Creating link #{} - {}::{} <-> {}::{}",
                    idx, link.node_a, link.int_a, link.node_b, link.int_b
                ),
                StatusKind::Progress,
            );

            tracing::info!(
                lab_id = %lab_id,
                link_num = idx,
                node_a = %link.node_a,
                int_a = %link.int_a,
                node_b = %link.node_b,
                int_b = %link.int_b,
                "Creating point-to-point link"
            );

            network::create_bridge(
                &bridge_a,
                &format!("{}-bridge-{}::{}", lab_id, link.node_a, link.int_a),
            )
            .await?;

            network::create_bridge(
                &bridge_b,
                &format!("{}-bridge-{}::{}", lab_id, link.node_b, link.int_b),
            )
            .await?;

            network::create_veth_pair(
                &veth_a,
                &veth_b,
                &format!("{}-veth-{}::{}", lab_id, link.node_a, link.int_a),
                &format!("{}-veth-{}::{}", lab_id, link.node_b, link.int_b),
            )
            .await?;

            network::enslave_to_bridge(&veth_a, &bridge_a).await?;
            network::enslave_to_bridge(&veth_b, &bridge_b).await?;

            tracing::debug!(
                lab_id = %lab_id,
                bridge_a = %bridge_a,
                bridge_b = %bridge_b,
                veth_a = %veth_a,
                veth_b = %veth_b,
                "Created link infrastructure"
            );
        }

        tracing::info!(
            lab_id = %lab_id,
            links_created = links_detailed.len(),
            "All point-to-point links created"
        );

        phases_completed.push("LinkCreation".to_string());

        // ========================================================================
        // PHASE 6: Docker Container Link Networks
        // ========================================================================
        let _ = progress.send_phase(
            data::UpPhase::ContainerNetworks,
            "Creating Docker networks for container links".to_string(),
        );

        tracing::info!(lab_id = %lab_id, "Creating Docker networks for container-connected bridges");

        let mut docker_net_count = 0;
        for link_data in &lab_link_data {
            let node_a_data = lab_node_data
                .iter()
                .find(|n| n.record.id == link_data.node_a.id)
                .ok_or_else(|| anyhow!("Node A not found in lab_node_data"))?;

            let node_b_data = lab_node_data
                .iter()
                .find(|n| n.record.id == link_data.node_b.id)
                .ok_or_else(|| anyhow!("Node B not found in lab_node_data"))?;

            if node_a_data.kind == data::NodeKind::Container {
                let docker_net_name =
                    format!("{}-etha{}-{}", node_a_data.name, link_data.index, lab_id);
                tracing::info!(
                    lab_id = %lab_id,
                    node = %node_a_data.name,
                    network = %docker_net_name,
                    bridge = %link_data.bridge_a,
                    "Creating Docker macvlan network"
                );
                container::create_docker_macvlan_network(
                    &docker_conn,
                    &link_data.bridge_a,
                    &docker_net_name,
                )
                .await?;
                docker_net_count += 1;
            }

            if node_b_data.kind == data::NodeKind::Container {
                let docker_net_name =
                    format!("{}-ethb{}-{}", node_b_data.name, link_data.index, lab_id);
                tracing::info!(
                    lab_id = %lab_id,
                    node = %node_b_data.name,
                    network = %docker_net_name,
                    bridge = %link_data.bridge_b,
                    "Creating Docker macvlan network"
                );
                container::create_docker_macvlan_network(
                    &docker_conn,
                    &link_data.bridge_b,
                    &docker_net_name,
                )
                .await?;
                docker_net_count += 1;
            }
        }

        // Create Docker macvlan bridge-mode networks for disabled interfaces on container nodes
        for nsd in &node_setup_data {
            let iso_network = match &nsd.isolated_network {
                Some(n) => n,
                None => continue,
            };

            // Only process container nodes
            let node_data = match lab_node_data.iter().find(|n| n.name == nsd.name) {
                Some(n) if n.kind == data::NodeKind::Container => n,
                _ => continue,
            };

            for iface in &nsd.interfaces {
                if !matches!(iface.data, data::NodeInterface::Disabled) {
                    continue;
                }
                let docker_net_name = format!("{}-iso{}-{}", node_data.name, iface.index, lab_id);
                tracing::info!(
                    lab_id = %lab_id,
                    node = %node_data.name,
                    interface = %iface.name,
                    network = %docker_net_name,
                    bridge = %iso_network.bridge_name,
                    "Creating isolated Docker macvlan bridge-mode network"
                );
                container::create_docker_macvlan_bridge_network(
                    &docker_conn,
                    &iso_network.bridge_name,
                    &docker_net_name,
                )
                .await?;
                docker_net_count += 1;
            }
        }

        tracing::info!(
            lab_id = %lab_id,
            networks_created = docker_net_count,
            "Docker macvlan networks created"
        );

        phases_completed.push("ContainerNetworks".to_string());

        // ========================================================================
        // PHASE 7: Shared Bridge Creation
        // ========================================================================
        let _ = progress.send_phase(
            data::UpPhase::SharedBridges,
            format!("Creating {} shared bridges", bridges_detailed.len()),
        );

        tracing::info!(
            lab_id = %lab_id,
            bridge_count = bridges_detailed.len(),
            "Creating shared bridges"
        );

        for bridge in bridges_detailed.iter() {
            let mut bridge_nodes = vec![];

            let _ = progress.send_status(
                format!(
                    "Creating shared bridge #{} - {} ({} connections)",
                    bridge.index,
                    bridge.manifest_name,
                    bridge.links.len()
                ),
                StatusKind::Progress,
            );

            tracing::info!(
                lab_id = %lab_id,
                bridge_num = bridge.index,
                bridge_name = %bridge.manifest_name,
                connections = bridge.links.len(),
                "Creating shared bridge"
            );

            network::create_bridge(&bridge.bridge_name, &bridge.libvirt_name).await?;

            tracing::debug!(
                lab_id = %lab_id,
                bridge = %bridge.bridge_name,
                libvirt_name = %bridge.libvirt_name,
                "Created bridge infrastructure"
            );

            for link in bridge.links.iter() {
                if let Some(node_data) = lab_node_data.iter().find(|n| n.name == link.node_name) {
                    bridge_nodes.push(db::get_node_id(&node_data.record)?);
                }
            }

            db::create_bridge(
                &db,
                bridge.index,
                bridge.bridge_name.clone(),
                bridge.libvirt_name.clone(),
                lab_record_id.clone(),
                bridge_nodes,
            )
            .await?;
        }

        tracing::info!(
            lab_id = %lab_id,
            bridges_created = bridges_detailed.len(),
            "All shared bridges created"
        );

        phases_completed.push("SharedBridges".to_string());

        // ========================================================================
        // PHASE 8: ZTP Configuration Generation
        // ========================================================================
        let _ = progress.send_phase(
            data::UpPhase::ZtpGeneration,
            "Generating ZTP configurations".to_string(),
        );

        tracing::info!(
            lab_id = %lab_id,
            total_nodes = container_nodes.len() + vm_nodes.len(),
            "Generating ZTP configurations"
        );

        // Create ZTP directories for TFTP-based ZTP (needed before VM ZTP generation)
        let ztp_dir = format!("{lab_dir}/{ZTP_DIR}");
        let tftp_dir = format!("{ztp_dir}/{TFTP_DIR}");
        util::create_dir(&ztp_dir)?;
        util::create_dir(&tftp_dir)?;

        // Generate per-lab TLS CA certificate
        let certs_dir = format!("{lab_dir}/{LAB_CERTS_DIR}");
        util::create_dir(&certs_dir)?;

        let ca_cert_path = Path::new(&certs_dir).join(LAB_CA_CERT_FILE);
        let ca_key_path = Path::new(&certs_dir).join(LAB_CA_KEY_FILE);

        let lab_ca = tls::generator::generate_lab_ca(
            &ca_cert_path,
            &ca_key_path,
            lab_id,
            LAB_CERT_VALIDITY_DAYS,
        )
        .context("Failed to generate lab CA certificate")?;

        let _ = progress.send_status(
            format!("Lab CA certificate generated for lab: {}", lab_id),
            StatusKind::Done,
        );

        // Container nodes ZTP generation
        for node in &mut container_nodes {
            // Decode base64 ztp_config if present
            if let Some(ref encoded) = node.ztp_config {
                let decoded = util::base64_decode(encoded).with_context(|| {
                    format!("Failed to decode ztp_config for node '{}'", node.name)
                })?;
                node.ztp_config = Some(decoded);
            }

            let node_data = node_ops::get_node_data(&node.name, &node_setup_data)?;
            let node_idx = node_data.index;
            let node_ip_idx = 10 + node_idx as u32;

            let node_image = get_node_image(&node.model, node.version.as_deref(), &node_images)?;
            let node_ipv4_address = util::get_ipv4_addr(&mgmt_net.v4.prefix, node_ip_idx)?;
            node.ipv4_address = Some(node_ipv4_address);

            // Assign IPv6 management address
            if let Some(ref v6) = mgmt_net.v6 {
                let addr = util::get_ipv6_addr(&v6.prefix, node_ip_idx)?;
                node.ipv6_address = Some(addr);
            }

            // Persist management IPs to the database
            if let Some(node_data) = lab_node_data.iter().find(|n| n.name == node.name) {
                let record_id = db::get_node_id(&node_data.record)?;
                db::update_node_mgmt_ipv4(&db, record_id.clone(), &node_ipv4_address.to_string())
                    .await?;
                if let Some(ipv6) = node.ipv6_address {
                    db::update_node_mgmt_ipv6(&db, record_id, &ipv6.to_string()).await?;
                }
            }

            // Generate per-node TLS certificate
            let node_cert_path = Path::new(&certs_dir).join(format!("{}.crt", node.name));
            let node_key_path = Path::new(&certs_dir).join(format!("{}.key", node.name));
            tls::generator::generate_node_certificate(
                &node_cert_path,
                &node_key_path,
                &lab_ca,
                &node.name,
                &node_ipv4_address.to_string(),
                LAB_CERT_VALIDITY_DAYS,
            )
            .with_context(|| {
                format!(
                    "Failed to generate TLS certificate for node '{}'",
                    node.name
                )
            })?;

            let cert_paths = node_ops::NodeCertPaths {
                ca_cert: ca_cert_path.to_string_lossy().to_string(),
                node_cert: node_cert_path.to_string_lossy().to_string(),
                node_key: node_key_path.to_string_lossy().to_string(),
            };

            let ztp_result = node_ops::generate_container_ztp(
                node,
                &node_image,
                &lab_dir,
                &sherpa_user,
                &dns,
                &mgmt_net,
                node_ipv4_address,
                &progress,
                Some(&cert_paths),
            )?;

            ztp_records.push(ztp_result.ztp_record);
        }

        // VM nodes ZTP generation, disk setup, and domain template building
        tracing::info!(
            lab_id = %lab_id,
            vm_count = vm_nodes.len(),
            "Generating VM ZTP configurations and domain templates"
        );

        for node in &mut vm_nodes {
            // Decode base64 ztp_config if present
            if let Some(ref encoded) = node.ztp_config {
                let decoded = util::base64_decode(encoded).with_context(|| {
                    format!("Failed to decode ztp_config for node '{}'", node.name)
                })?;
                node.ztp_config = Some(decoded);
            }

            let node_data = node_ops::get_node_data(&node.name, &node_setup_data)?;
            let node_idx = node_data.index;
            let node_ip_idx = 10 + node_idx as u32;

            let node_image = get_node_image(&node.model, node.version.as_deref(), &node_images)?;
            let node_ipv4_address = util::get_ipv4_addr(&mgmt_net.v4.prefix, node_ip_idx)?;
            node.ipv4_address = Some(node_ipv4_address);

            // Assign IPv6 management address
            if let Some(ref v6) = mgmt_net.v6 {
                let addr = util::get_ipv6_addr(&v6.prefix, node_ip_idx)?;
                node.ipv6_address = Some(addr);
            }

            // Persist management IPs to the database
            if let Some(node_data) = lab_node_data.iter().find(|n| n.name == node.name) {
                let record_id = db::get_node_id(&node_data.record)?;
                db::update_node_mgmt_ipv4(&db, record_id.clone(), &node_ipv4_address.to_string())
                    .await?;
                if let Some(ipv6) = node.ipv6_address {
                    db::update_node_mgmt_ipv6(&db, record_id, &ipv6.to_string()).await?;
                }
            }

            // Generate per-node TLS certificate
            let node_cert_path = Path::new(&certs_dir).join(format!("{}.crt", node.name));
            let node_key_path = Path::new(&certs_dir).join(format!("{}.key", node.name));
            tls::generator::generate_node_certificate(
                &node_cert_path,
                &node_key_path,
                &lab_ca,
                &node.name,
                &node_ipv4_address.to_string(),
                LAB_CERT_VALIDITY_DAYS,
            )
            .with_context(|| {
                format!(
                    "Failed to generate TLS certificate for node '{}'",
                    node.name
                )
            })?;

            let cert_paths = node_ops::NodeCertPaths {
                ca_cert: ca_cert_path.to_string_lossy().to_string(),
                node_cert: node_cert_path.to_string_lossy().to_string(),
                node_key: node_key_path.to_string_lossy().to_string(),
            };

            // Generate VM ZTP configuration, disks, and clone list
            let ztp_result = node_ops::generate_vm_ztp(
                node,
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
                None,
                Some(&cert_paths),
            )?;

            // Persist management MAC to the database
            if let Some(node_data) = lab_node_data.iter().find(|n| n.name == node.name) {
                let record_id = db::get_node_id(&node_data.record)?;
                db::update_node_mgmt_mac(&db, record_id, &ztp_result.mac_address).await?;
            }

            ztp_records.push(ztp_result.ztp_record);
            clone_disks.extend(ztp_result.clone_disks);

            // Build interfaces list
            let mut interfaces: Vec<data::Interface> = vec![];
            for interface in node_data.interfaces.iter() {
                match &interface.data {
                    data::NodeInterface::Management => {
                        interfaces.push(data::Interface {
                            name: util::dasher(&node_image.management_interface.to_string()),
                            num: interface.index,
                            mtu: node_image.interface_mtu,
                            mac_address: ztp_result.mac_address.clone(),
                            connection_type: data::ConnectionTypes::Management,
                            interface_connection: None,
                        });
                    }
                    data::NodeInterface::Reserved => {
                        interfaces.push(data::Interface {
                            name: format!("int{}", interface.index),
                            num: interface.index,
                            mtu: node_image.interface_mtu,
                            mac_address: util::random_mac(KVM_OUI),
                            connection_type: data::ConnectionTypes::Reserved,
                            interface_connection: None,
                        });
                    }
                    data::NodeInterface::Bridge(bridge) => {
                        interfaces.push(data::Interface {
                            name: bridge.name.clone(),
                            num: interface.index,
                            mtu: node_image.interface_mtu,
                            mac_address: util::random_mac(KVM_OUI),
                            connection_type: data::ConnectionTypes::PrivateBridge,
                            interface_connection: None,
                        });
                    }
                    data::NodeInterface::Peer(peer) => {
                        let local_id = peer.this_node_index as u8;
                        let source_id = peer.peer_node_index as u8;
                        let bridge_name = match peer.this_side {
                            data::PeerSide::A => {
                                format!("{}a{}-{}", BRIDGE_PREFIX, peer.link_index, lab_id)
                            }
                            data::PeerSide::B => {
                                format!("{}b{}-{}", BRIDGE_PREFIX, peer.link_index, lab_id)
                            }
                        };
                        let interface_connection = data::InterfaceConnection {
                            local_id: peer.this_node_index,
                            local_port: util::id_to_port(local_id),
                            local_loopback: util::get_ip(&loopback_subnet, local_id).to_string(),
                            source_id: peer.peer_node_index,
                            source_port: util::id_to_port(source_id),
                            source_loopback: util::get_ip(&loopback_subnet, source_id).to_string(),
                        };
                        interfaces.push(data::Interface {
                            name: bridge_name,
                            num: interface.index,
                            mtu: node_image.interface_mtu,
                            mac_address: util::random_mac(KVM_OUI),
                            connection_type: data::ConnectionTypes::PeerBridge,
                            interface_connection: Some(interface_connection),
                        });
                    }
                    data::NodeInterface::Disabled => {
                        interfaces.push(data::Interface {
                            name: util::dasher(&util::interface_from_idx(
                                &node.model,
                                interface.index,
                            )?),
                            num: interface.index,
                            mtu: node_image.interface_mtu,
                            mac_address: util::random_mac(KVM_OUI),
                            connection_type: data::ConnectionTypes::Disabled,
                            interface_connection: None,
                        });
                    }
                }
            }

            // Get network names
            let management_network = node_data.management_network.clone();
            let isolated_network = node_data
                .isolated_network
                .clone()
                .ok_or_else(|| anyhow!("Isolated network not found for VM node: {}", node.name))?;
            let reserved_network = node_data
                .reserved_network
                .as_ref()
                .map(|net| net.network_name.clone())
                .unwrap_or_default();

            // Build domain template
            let domain = node_ops::build_domain_template(
                node,
                &node_image,
                lab_id,
                &config.qemu_bin,
                ztp_result.disks,
                interfaces,
                ztp_result.qemu_commands,
                util::get_ip(&loopback_subnet, node_idx as u8).to_string(),
                management_network,
                isolated_network.network_name,
                reserved_network,
            );
            domains.push(domain);
        }

        phases_completed.push("ZtpGeneration".to_string());

        // ========================================================================
        // PHASE 9: Sherpa Router & ZTP File Creation
        // ========================================================================
        let _ = progress.send_phase(
            data::UpPhase::BootContainers,
            "Creating boot containers and ZTP files".to_string(),
        );

        tracing::info!(lab_id = %lab_id, "Creating ZTP boot infrastructure");

        // Create remaining ZTP directories (ztp_dir and tftp_dir already created in Phase 8)
        let ztp_configs_dir = format!("{ztp_dir}/{NODE_CONFIGS_DIR}");
        let dnsmasq_dir = format!("{ztp_dir}/{DNSMASQ_DIR}");
        util::create_dir(&ztp_configs_dir)?;
        util::create_dir(&dnsmasq_dir)?;

        tracing::debug!(
            lab_id = %lab_id,
            ztp_dir = %ztp_dir,
            tftp_dir = %tftp_dir,
            dnsmasq_dir = %dnsmasq_dir,
            "Created ZTP directories"
        );

        // Create dnsmasq config
        let dnsmaq_template = template::DnsmasqTemplate {
            tftp_server_ipv4: mgmt_net.v4.boot_server.to_string(),
            gateway_ipv4: mgmt_net.v4.first.to_string(),
            dhcp_start: util::get_ipv4_addr(&mgmt_net.v4.prefix, 10)?.to_string(),
            dhcp_end: util::get_ipv4_addr(&mgmt_net.v4.prefix, 254)?.to_string(),
            gateway_ipv6: mgmt_net.v6.as_ref().map(|v6| v6.first.to_string()),
            dhcp6_start: mgmt_net
                .v6
                .as_ref()
                .map(|v6| util::get_ipv6_addr(&v6.prefix, 10).map(|a| a.to_string()))
                .transpose()?,
            dhcp6_end: mgmt_net
                .v6
                .as_ref()
                .map(|v6| util::get_ipv6_addr(&v6.prefix, 254).map(|a| a.to_string()))
                .transpose()?,
            dns_ipv6: mgmt_net.v6.as_ref().map(|v6| v6.boot_server.to_string()),
            ztp_records: ztp_records.clone(),
        };
        let dnsmasq_rendered_template = dnsmaq_template.render()?;
        util::create_file(
            &format!("{dnsmasq_dir}/{DNSMASQ_CONFIG_FILE}"),
            dnsmasq_rendered_template,
        )?;
        util::create_file(
            &format!("{dnsmasq_dir}/{DNSMASQ_LEASES_FILE}"),
            "".to_string(),
        )?;

        // Create boot container (tftp_dir already created in Phase 8)
        let configs_dir = format!("{ztp_dir}/{NODE_CONFIGS_DIR}");

        let dnsmasq_env_dns1 = format!("DNS1={}", mgmt_net.v4.first);
        let dnsmasq_env_dns2 = "DNS2=".to_string();
        let boot_server_ipv4 = mgmt_net.v4.boot_server.to_string();

        let webdir_config_volume = format!("{configs_dir}:/opt/{ZTP_DIR}/{NODE_CONFIGS_DIR}");
        let dnsmasq_env_vars = vec![dnsmasq_env_dns1, dnsmasq_env_dns2];
        let dnsmasq_config_volume =
            format!("{dnsmasq_dir}/{DNSMASQ_CONFIG_FILE}:/etc/{DNSMASQ_CONFIG_FILE}");
        let dnsmasq_tftp_volume = format!("{tftp_dir}:/opt/{ZTP_DIR}/{TFTP_DIR}");
        let dnsmasq_volumes = vec![
            dnsmasq_config_volume,
            dnsmasq_tftp_volume,
            webdir_config_volume,
        ];
        let dnsmasq_capabilities = vec!["NET_ADMIN"];

        let management_network_attachment = data::ContainerNetworkAttachment {
            name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
            ipv4_address: Some(boot_server_ipv4.clone()),
            ipv6_address: None,
            linux_interface_name: None,
            admin_down: false,
        };

        tracing::info!(
            lab_id = %lab_id,
            container = %format!("{CONTAINER_DNSMASQ_NAME}-{lab_id}"),
            boot_server_ip = %boot_server_ipv4,
            "Starting dnsmasq boot container"
        );

        let is_running = container::run_container(
            &docker_conn,
            &format!("{CONTAINER_DNSMASQ_NAME}-{lab_id}"),
            CONTAINER_DNSMASQ_REPO,
            dnsmasq_env_vars,
            dnsmasq_volumes,
            dnsmasq_capabilities,
            management_network_attachment,
            vec![],
            vec![],
            false,
            None,
            None,
        )
        .await?;

        if !is_running {
            anyhow::bail!(
                "dnsmasq boot container {CONTAINER_DNSMASQ_NAME}-{lab_id} is not in running state after start"
            );
        }

        phases_completed.push("Sherpa Router".to_string());

        // ========================================================================
        // PHASE 10: Disk Cloning (For VMs)
        // ========================================================================
        let _ = progress.send_phase(data::UpPhase::DiskCloning, "Cloning VM disks".to_string());

        node_ops::clone_node_disks(qemu_conn.clone(), clone_disks, lab_id, &progress).await?;

        phases_completed.push("DiskCloning".to_string());

        // ========================================================================
        // PHASE 11: VM Creation
        // ========================================================================
        let _ = progress.send_phase(data::UpPhase::VmCreation, "Creating VMs".to_string());

        if !domains.is_empty() {
            let vm_count = domains.len();
            let _ = progress.send_status(
                format!("Creating {} VMs in parallel", vm_count),
                StatusKind::Progress,
            );

            let tasks: Vec<_> = domains
                .into_iter()
                .map(|domain| {
                    let conn = Arc::clone(&qemu_conn);
                    let progress_clone = progress.clone();
                    tokio::task::spawn(async move {
                        node_ops::create_vm(conn, domain, &progress_clone).await
                    })
                })
                .collect();

            for task in tasks {
                task.await.context("VM creation task failed")??;
            }

            let _ =
                progress.send_status("All VMs created successfully".to_string(), StatusKind::Done);
        } else {
            let _ = progress.send_status("No VMs to create".to_string(), StatusKind::Info);
        }

        phases_completed.push("VmCreation".to_string());

        // ========================================================================
        // PHASE 12: SSH Config & Network Map Building
        // ========================================================================
        let _ = progress.send_phase(
            data::UpPhase::SshConfig,
            "Generating SSH config".to_string(),
        );

        tracing::info!(lab_id = %lab_id, "Generating SSH configuration");

        // Load server config to get server_ipv4
        let config_contents = util::load_file(SHERPA_CONFIG_FILE_PATH)
            .context("Failed to load sherpa.toml config")?;
        let config: data::Config =
            toml::from_str(&config_contents).context("Failed to parse sherpa.toml config")?;

        // Use client's username for ProxyJump (same user that initiated the lab creation)
        let proxy_user = current_user;

        tracing::debug!(
            lab_id = %lab_id,
            proxy_user = %proxy_user,
            server_ip = %config.server_ipv4,
            "SSH config parameters"
        );

        // Generate SSH config file with ProxyJump
        let ssh_config_template = template::SshConfigTemplate {
            ztp_records: ztp_records.clone(),
            proxy_user: proxy_user.to_string(),
            server_ipv4: config.server_ipv4.to_string(),
        };
        let ssh_config_content = ssh_config_template.render()?;
        let ssh_config_path = format!("{lab_dir}/{SHERPA_SSH_CONFIG_FILE}");
        util::create_file(&ssh_config_path, ssh_config_content.clone())?;
        tracing::info!(
            lab_id = %lab_id,
            config_path = %ssh_config_path,
            "Created SSH config file"
        );
        let _ = progress.send_status("SSH config file created".to_string(), StatusKind::Done);

        // Read SSH private key for transfer to client
        let ssh_private_key = util::load_file(SHERPA_SSH_PRIVATE_KEY_PATH)
            .context("Failed to read SSH private key")?;
        tracing::debug!(
            lab_id = %lab_id,
            key_path = %SHERPA_SSH_PRIVATE_KEY_PATH,
            "Loaded SSH private key"
        );
        let _ = progress.send_status("SSH private key loaded".to_string(), StatusKind::Done);

        // Build container network mappings from the full interface list.
        // This ensures all defined data interfaces appear in model-index order,
        // with linked interfaces active and disabled interfaces admin-down.
        let mut container_link_networks: HashMap<String, Vec<data::ContainerNetworkAttachment>> =
            HashMap::new();

        // Step 1: Build a lookup of (node_name, interface_name) -> docker_network_name
        // from the link data for container nodes.
        let mut container_link_net_lookup: HashMap<(String, String), String> = HashMap::new();
        for link_data in &lab_link_data {
            let node_a_data = lab_node_data
                .iter()
                .find(|n| n.record.id == link_data.node_a.id)
                .ok_or_else(|| {
                    anyhow!(
                        "Node A not found in lab_node_data for link index {}",
                        link_data.index
                    )
                })?;
            let node_b_data = lab_node_data
                .iter()
                .find(|n| n.record.id == link_data.node_b.id)
                .ok_or_else(|| {
                    anyhow!(
                        "Node B not found in lab_node_data for link index {}",
                        link_data.index
                    )
                })?;

            if node_a_data.kind == data::NodeKind::Container {
                let docker_net_name =
                    format!("{}-etha{}-{}", node_a_data.name, link_data.index, lab_id);
                container_link_net_lookup.insert(
                    (node_a_data.name.clone(), link_data.int_a.clone()),
                    docker_net_name,
                );
            }

            if node_b_data.kind == data::NodeKind::Container {
                let docker_net_name =
                    format!("{}-ethb{}-{}", node_b_data.name, link_data.index, lab_id);
                container_link_net_lookup.insert(
                    (node_b_data.name.clone(), link_data.int_b.clone()),
                    docker_net_name,
                );
            }
        }

        // Step 2: Walk each container node's interfaces in index order.
        // For each data interface, produce a ContainerNetworkAttachment with
        // the correct docker network name and linux_interface_name.
        for nsd in &node_setup_data {
            let node_data = match lab_node_data.iter().find(|n| n.name == nsd.name) {
                Some(n) if n.kind == data::NodeKind::Container => n,
                _ => continue,
            };

            let attachments: Vec<data::ContainerNetworkAttachment> = nsd
                .interfaces
                .iter()
                .filter_map(|iface| {
                    match &iface.data {
                        data::NodeInterface::Peer(_) | data::NodeInterface::Bridge(_) => {
                            // Linked interface — look up docker network name
                            let docker_net_name = container_link_net_lookup
                                .get(&(nsd.name.clone(), iface.name.clone()))?;
                            let linux_interface_name =
                                if node_data.model == data::NodeModel::NokiaSrlinux {
                                    util::srlinux_to_linux_interface(&iface.name).ok()
                                } else {
                                    None
                                };
                            Some(data::ContainerNetworkAttachment {
                                name: docker_net_name.clone(),
                                ipv4_address: None,
                                ipv6_address: None,
                                linux_interface_name,
                                admin_down: false,
                            })
                        }
                        data::NodeInterface::Disabled => {
                            // Disabled interface — use isolated network
                            let docker_net_name =
                                format!("{}-iso{}-{}", node_data.name, iface.index, lab_id);
                            let linux_interface_name =
                                util::interface_from_idx(&node_data.model, iface.index)
                                    .ok()
                                    .and_then(|name| {
                                        if node_data.model == data::NodeModel::NokiaSrlinux {
                                            util::srlinux_to_linux_interface(&name).ok()
                                        } else {
                                            Some(name)
                                        }
                                    });
                            Some(data::ContainerNetworkAttachment {
                                name: docker_net_name,
                                ipv4_address: None,
                                ipv6_address: None,
                                linux_interface_name,
                                admin_down: true,
                            })
                        }
                        // Skip Management and Reserved
                        _ => None,
                    }
                })
                .collect();

            container_link_networks.insert(nsd.name.clone(), attachments);
        }

        tracing::info!(
            lab_id = %lab_id,
            container_count = container_link_networks.len(),
            "Built container network mappings"
        );

        phases_completed.push("SshConfig".to_string());

        // ========================================================================
        // PHASE 13: Node Readiness Polling
        // ========================================================================
        let ready_timeout_secs = manifest.ready_timeout.unwrap_or(READINESS_TIMEOUT);
        let _ = progress.send_phase(
            data::UpPhase::NodeReadiness,
            format!(
                "Waiting for {} nodes to become ready (up to {} seconds)",
                container_nodes.len() + vm_nodes.len(),
                ready_timeout_secs
            ),
        );

        let start_time_readiness = Instant::now();
        let readiness_timer = std::time::Instant::now();
        let timeout = Duration::from_secs(ready_timeout_secs);
        let mut connected_nodes = std::collections::HashSet::new();
        let mut node_info_list = vec![];

        let all_lab_nodes = [
            container_nodes.clone(),
            unikernel_nodes.clone(),
            vm_nodes.clone(),
        ]
        .concat();
        let total_lab_nodes = all_lab_nodes.len();

        let node_names = all_lab_nodes
            .iter()
            .map(|x| x.name.as_str())
            .collect::<Vec<&str>>()
            .join(", ");

        tracing::debug!(
            lab_id = %lab_id,
            nodes = %node_names,
            "Waiting for nodes"
        );

        let _ = progress.send_status(
            format!("Waiting for nodes: {}", node_names),
            StatusKind::Waiting,
        );

        // Handle nodes with skip_ready_check enabled
        for node in &all_lab_nodes {
            if node.skip_ready_check.unwrap_or(false) {
                tracing::info!(
                    lab_id = %lab_id,
                    node_name = %node.name,
                    "Skipping ready check for node"
                );
                let _ = progress.send_status(
                    format!("Node {} - Ready check skipped", node.name),
                    StatusKind::Done,
                );
                connected_nodes.insert(node.name.clone());
                let kind = lab_node_data
                    .iter()
                    .find(|n| n.name == node.name)
                    .map(|n| format!("{:?}", n.kind))
                    .unwrap_or_else(|| "Unknown".to_string());
                node_info_list.push(data::NodeInfo {
                    name: node.name.clone(),
                    kind,
                    model: node.model,
                    status: NodeState::Unknown,
                    ip_address: node.ipv4_address.map(|i| i.to_string()),
                    ssh_port: None,
                });
            }
        }

        tracing::info!(
            lab_id = %lab_id,
            total_nodes = total_lab_nodes,
            containers = container_nodes.len(),
            vms = vm_nodes.len(),
            unikernels = unikernel_nodes.len(),
            timeout_secs = ready_timeout_secs,
            "Starting node readiness polling"
        );

        while start_time_readiness.elapsed() < timeout && connected_nodes.len() < total_lab_nodes {
            // Start containers
            for container in &container_nodes {
                if connected_nodes.contains(&container.name) {
                    continue;
                }

                let mgmt_ipv4 = container.ipv4_address.map(|i| i.to_string());
                let container_name = format!("{}-{}", container.name, lab_id);

                // Extract image and version with proper error handling
                let container_image_name = container.image.as_ref().ok_or_else(|| {
                    anyhow!("Container image not set for node: {}", container.name)
                })?;

                let container_version = container.version.as_ref().ok_or_else(|| {
                    anyhow!("Container version not set for node: {}", container.name)
                })?;

                let container_image = format!("{}:{}", container_image_name, container_version);
                let privileged = container.privileged.unwrap_or(false);
                let shm_size = container.shm_size;
                let env_vars = container.environment_variables.clone().unwrap_or_default();
                let commands = container.commands.clone().unwrap_or_default();
                let user = container.user.clone();
                let volumes = if let Some(volumes) = container.volumes.clone() {
                    volumes
                        .iter()
                        .map(|v| format!("{}:{}", v.src, v.dst))
                        .collect()
                } else {
                    vec![]
                };

                let management_network_attachment = data::ContainerNetworkAttachment {
                    name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
                    ipv4_address: mgmt_ipv4.clone(),
                    ipv6_address: None,
                    linux_interface_name: None,
                    admin_down: false,
                };

                let mut additional_networks = vec![];
                if let Some(link_networks) = container_link_networks.get(&container.name) {
                    additional_networks.extend_from_slice(link_networks);
                }

                let is_running = node_ops::start_container_node(
                    &docker_conn,
                    &container_name,
                    &container_image,
                    env_vars,
                    volumes,
                    management_network_attachment,
                    additional_networks,
                    commands,
                    privileged,
                    shm_size,
                    user,
                    container.model,
                    &progress,
                )
                .await?;

                if !is_running {
                    continue;
                }

                connected_nodes.insert(container.name.clone());

                // Update node state in DB to Running
                if let Some(node_data) = lab_node_data.iter().find(|n| n.name == container.name) {
                    let record_id = db::get_node_id(&node_data.record)?;
                    db::update_node_state(&db, record_id, NodeState::Running).await?;
                }

                node_info_list.push(data::NodeInfo {
                    name: container.name.clone(),
                    kind: "Container".to_string(),
                    model: container.model,
                    status: NodeState::Running,
                    ip_address: mgmt_ipv4,
                    ssh_port: Some(SSH_PORT),
                });
            }

            // Check VMs for readiness
            for vm in &vm_nodes {
                if connected_nodes.contains(&vm.name) {
                    continue;
                }

                if let Some(vm_data) = ztp_records.iter().find(|x| x.node_name == vm.name) {
                    match node_ops::check_node_ready_ssh(
                        &vm_data.ipv4_address.to_string(),
                        SSH_PORT,
                    )? {
                        true => {
                            tracing::info!(
                                lab_id = %lab_id,
                                node_name = %vm.name,
                                node_kind = "VirtualMachine",
                                ipv4 = %vm_data.ipv4_address,
                                "VM ready (SSH accessible)"
                            );
                            let _ = progress.send_status(
                                format!("Node {} - Ready (SSH accessible)", vm.name),
                                StatusKind::Done,
                            );
                            connected_nodes.insert(vm.name.clone());

                            // Update node state in DB to Running
                            if let Some(node_data) =
                                lab_node_data.iter().find(|n| n.name == vm.name)
                            {
                                let record_id = db::get_node_id(&node_data.record)?;
                                db::update_node_state(&db, record_id, NodeState::Running).await?;
                            }

                            node_info_list.push(data::NodeInfo {
                                name: vm.name.clone(),
                                kind: "VirtualMachine".to_string(),
                                model: vm.model,
                                status: NodeState::Running,
                                ip_address: Some(vm_data.ipv4_address.to_string()),
                                ssh_port: Some(SSH_PORT),
                            });
                        }
                        false => {
                            tracing::debug!(
                                lab_id = %lab_id,
                                node_name = %vm.name,
                                ipv4 = %vm_data.ipv4_address,
                                "Waiting for VM SSH connection"
                            );
                            let _ = progress.send_status(
                                format!("Node {} - Waiting for SSH", vm.name),
                                StatusKind::Waiting,
                            );
                        }
                    }
                }
            }

            if connected_nodes.len() < total_lab_nodes {
                tokio::time::sleep(Duration::from_secs(READINESS_SLEEP)).await;
            }
        }

        let readiness_elapsed = readiness_timer.elapsed().as_secs();

        if connected_nodes.len() == total_lab_nodes {
            tracing::info!(
                lab_id = %lab_id,
                nodes_ready = connected_nodes.len(),
                total_nodes = total_lab_nodes,
                duration_secs = readiness_elapsed,
                "All nodes ready"
            );
            let _ = progress.send_status("All nodes are ready!".to_string(), StatusKind::Done);
        } else {
            tracing::warn!(
                lab_id = %lab_id,
                nodes_ready = connected_nodes.len(),
                total_nodes = total_lab_nodes,
                duration_secs = readiness_elapsed,
                timeout_secs = ready_timeout_secs,
                "Timeout reached - not all nodes ready"
            );
            let _ = progress.send_status(
                format!(
                    "Timeout reached. {} of {} nodes are ready.",
                    connected_nodes.len(),
                    total_lab_nodes
                ),
                StatusKind::Waiting,
            );
            for node in &all_lab_nodes {
                if !connected_nodes.contains(&node.name) {
                    tracing::warn!(
                        lab_id = %lab_id,
                        node_name = %node.name,
                        "Node not ready after timeout"
                    );
                    errors.push(data::UpError {
                        phase: "NodeReadiness".to_string(),
                        message: format!("Node {} did not become ready", node.name),
                        is_critical: false,
                    });
                }
            }
        }

        phases_completed.push("NodeReadiness".to_string());

        // ========================================================================
        // Build Response
        // ========================================================================

        let summary = data::UpSummary {
            containers_created: container_nodes.len(),
            vms_created: vm_nodes.len(),
            unikernels_created: unikernel_nodes.len(),
            networks_created: 1 + bridges_detailed.len(), // management + shared bridges
            bridges_created: links_detailed.len() * 2,    // 2 bridges per p2p link
            interfaces_created: lab_link_data.len() * 2,  // 2 veth interfaces per link
        };

        let critical_errors = errors.iter().filter(|e| e.is_critical).count();
        let warnings = errors.iter().filter(|e| !e.is_critical).count();
        let success = critical_errors == 0;
        let total_time = start_time.elapsed().as_secs();

        let response = data::UpResponse {
            success,
            lab_info: lab_info.clone(),
            total_time_secs: total_time,
            phases_completed: phases_completed.clone(),
            summary: summary.clone(),
            nodes: node_info_list,
            errors: errors.clone(),
            ssh_config: ssh_config_content,
            ssh_private_key,
        };

        tracing::info!(
            lab_id = %lab_id,
            lab_name = %manifest.name,
            success = success,
            total_time_secs = total_time,
            containers_created = summary.containers_created,
            vms_created = summary.vms_created,
            unikernels_created = summary.unikernels_created,
            networks_created = summary.networks_created,
            bridges_created = summary.bridges_created,
            interfaces_created = summary.interfaces_created,
            critical_errors = critical_errors,
            warnings = warnings,
            phases_completed = phases_completed.len(),
            "Lab creation completed"
        );

        Ok(response)
    }; // end resource_creation async block

    match resource_creation.await {
        Ok(response) => Ok(response),
        Err(e) => {
            tracing::error!(
                lab_id = %lab_id,
                error = ?e,
                "Lab creation failed after resource creation began, starting auto-cleanup"
            );
            let _ = progress.send_status(
                "Lab creation failed, cleaning up resources...".to_string(),
                StatusKind::Info,
            );

            match clean::clean_lab(lab_id, state).await {
                Ok(clean_response) => {
                    if clean_response.success {
                        tracing::info!(lab_id = %lab_id, "Auto-cleanup completed successfully");
                        let _ = progress.send_status(
                            "Cleanup completed successfully".to_string(),
                            StatusKind::Done,
                        );
                    } else {
                        tracing::warn!(
                            lab_id = %lab_id,
                            errors = ?clean_response.errors,
                            "Auto-cleanup completed with errors"
                        );
                        let _ = progress.send_status(
                            "Cleanup completed with some errors".to_string(),
                            StatusKind::Info,
                        );
                    }
                }
                Err(clean_err) => {
                    tracing::error!(
                        lab_id = %lab_id,
                        error = ?clean_err,
                        "Auto-cleanup failed"
                    );
                    let _ = progress
                        .send_status(format!("Cleanup failed: {clean_err}"), StatusKind::Info);
                }
            }

            Err(e)
        }
    }
}
