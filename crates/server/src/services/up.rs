// Server-side implementation of the lab startup operation
// This is a port of the client's up.rs command with streaming progress support

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use askama::Template;
use serde_json;

use crate::daemon::state::AppState;
use crate::services::progress::ProgressSender;

use shared::data;
use shared::data::NodeState;
use shared::konst::{
    ARISTA_CEOS_ZTP_VOLUME_MOUNT, BRIDGE_PREFIX, CISCO_ASAV_ZTP_CONFIG, CISCO_FTDV_ZTP_CONFIG,
    CISCO_IOSV_ZTP_CONFIG, CISCO_IOSXE_ZTP_CONFIG, CISCO_IOSXR_ZTP_CONFIG, CISCO_ISE_ZTP_CONFIG,
    CISCO_NXOS_ZTP_CONFIG, CLOUD_INIT_META_DATA, CLOUD_INIT_NETWORK_CONFIG, CLOUD_INIT_USER_DATA,
    CONTAINER_ARISTA_CEOS_COMMANDS, CONTAINER_ARISTA_CEOS_ENV_VARS, CONTAINER_ARISTA_CEOS_REPO,
    CONTAINER_DISK_NAME, CONTAINER_DNSMASQ_NAME, CONTAINER_DNSMASQ_REPO,
    CONTAINER_NOKIA_SRLINUX_COMMANDS, CONTAINER_NOKIA_SRLINUX_ENV_VARS,
    CONTAINER_NOKIA_SRLINUX_REPO, CONTAINER_SURREAL_DB_COMMANDS, CONTAINER_SURREAL_DB_REPO,
    CUMULUS_ZTP, DNSMASQ_CONFIG_FILE, DNSMASQ_DIR, DNSMASQ_LEASES_FILE, JUNIPER_ZTP_CONFIG,
    JUNIPER_ZTP_CONFIG_TGZ, KVM_OUI, LAB_FILE_NAME, NODE_CONFIGS_DIR, READINESS_SLEEP,
    READINESS_TIMEOUT, SHERPA_BASE_DIR, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_EXT4_500MB,
    SHERPA_BLANK_DISK_FAT32, SHERPA_BLANK_DISK_IOSV, SHERPA_BLANK_DISK_ISE,
    SHERPA_BLANK_DISK_JUNOS, SHERPA_CONFIG_DIR, SHERPA_CONFIG_FILE, SHERPA_DB_NAME,
    SHERPA_DB_NAMESPACE, SHERPA_DB_PORT, SHERPA_DB_SERVER, SHERPA_DOMAIN_NAME,
    SHERPA_ISOLATED_NETWORK_BRIDGE_PREFIX, SHERPA_ISOLATED_NETWORK_NAME, SHERPA_LABS_DIR,
    SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX, SHERPA_MANAGEMENT_NETWORK_NAME, SHERPA_PASSWORD,
    SHERPA_PASSWORD_HASH, SHERPA_RESERVED_NETWORK_BRIDGE_PREFIX, SHERPA_RESERVED_NETWORK_NAME,
    SHERPA_SSH_CONFIG_FILE, SHERPA_SSH_DIR, SHERPA_SSH_PRIVATE_KEY_FILE, SHERPA_STORAGE_POOL_PATH,
    SHERPA_USERNAME, SSH_PORT, TELNET_PORT, TFTP_DIR, VETH_PREFIX, ZTP_DIR, ZTP_ISO, ZTP_JSON,
};
use shared::util;
// use topology::{self, BridgeDetailed};

// ============================================================================
// ============================================================================
// Helper Functions (ported from client)
// ============================================================================

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
    manifest_nodes: &Vec<topology::NodeExpanded>,
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
                    node_model: node.model.clone(),
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

fn node_isolated_network_data(
    node_name: &str,
    node_index: u16,
    lab_id: &str,
) -> data::LabIsolatedNetwork {
    data::LabIsolatedNetwork {
        network_name: format!("{}-{}-{}", SHERPA_ISOLATED_NETWORK_NAME, node_name, lab_id),
        bridge_name: format!(
            "{}{}-{}",
            SHERPA_ISOLATED_NETWORK_BRIDGE_PREFIX, node_index, lab_id
        ),
    }
}

fn node_reserved_network_data(
    node_name: &str,
    node_index: u16,
    lab_id: &str,
) -> data::LabReservedNetwork {
    data::LabReservedNetwork {
        network_name: format!("{}-{}-{}", SHERPA_RESERVED_NETWORK_NAME, node_name, lab_id),
        bridge_name: format!(
            "{}{}-{}",
            SHERPA_RESERVED_NETWORK_BRIDGE_PREFIX, node_index, lab_id
        ),
    }
}

fn get_node_data(node_name: &str, data: &Vec<data::NodeSetupData>) -> Result<data::NodeSetupData> {
    Ok(data
        .iter()
        .find(|x| x.name == node_name)
        .ok_or_else(|| anyhow!("Node setup data not found for node: {}", node_name))?
        .clone())
}

/// Process manifest nodes into expanded format with indices assigned
fn process_manifest_nodes(manifest_nodes: &[topology::Node]) -> Vec<topology::NodeExpanded> {
    manifest_nodes
        .iter()
        .enumerate()
        .map(|(idx, node)| topology::NodeExpanded {
            name: node.name.clone(),
            model: node.model.clone(),
            index: idx as u16 + 1,
            version: node.version.clone(),
            memory: node.memory,
            cpu_count: node.cpu_count,
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
            let device_model = device.model.clone();
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

/// Get node configuration from a list of node configs
fn get_node_config(
    node_model: &data::NodeModel,
    data: &[data::NodeConfig],
) -> Result<data::NodeConfig> {
    Ok(data
        .iter()
        .find(|x| &x.model == node_model && x.default)
        .ok_or_else(|| anyhow!("Default node config not found for model: {}", node_model))?
        .clone())
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
    progress.send_phase(data::UpPhase::Setup, "Initializing connections".to_string())?;

    tracing::info!(lab_id = %lab_id, lab_name = %manifest.name, "Connecting to lab infrastructure services");

    let sherpa_user = util::sherpa_user().context("Failed to get sherpa user")?;
    let lab_dir = format!("{SHERPA_BASE_DIR}/{SHERPA_LABS_DIR}/{lab_id}");
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
    let db = db::connect(
        SHERPA_DB_SERVER,
        SHERPA_DB_PORT,
        SHERPA_DB_NAMESPACE,
        SHERPA_DB_NAME,
    )
    .await
    .context("Failed to connect to database")?;
    tracing::info!(lab_id = %lab_id, "Connected to SurrealDB database");

    tracing::debug!(lab_id = %lab_id, lab_dir = %lab_dir, user = %current_user, "Lab environment prepared");

    let db_user = db::get_user(&db, &current_user)
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

    progress.send_status("Loading configuration".to_string())?;
    let config = state.config.clone();

    // Bulk fetch all node configs from database
    let node_configs = db::list_node_configs(&db)
        .await
        .context("Failed to list node configs from database")?;

    phases_completed.push("Setup".to_string());

    // ========================================================================
    // PHASE 2: Manifest Validation
    // ========================================================================
    progress.send_phase(
        data::UpPhase::ManifestValidation,
        "Validating manifest structure".to_string(),
    )?;

    tracing::info!(
        lab_id = %lab_id,
        lab_name = %manifest.name,
        "Validating lab manifest"
    );

    tracing::debug!(lab_id = %lab_id, node_configs = node_configs.len(), "Fetched node configs from database");

    // Device Validators (CRITICAL ERROR - fail fast on validation failure)
    validate::check_duplicate_device(&manifest.nodes)
        .context("Manifest validation failed: duplicate devices")?;

    // Version & Image Validators (CRITICAL ERROR - fail fast on validation failure)
    // Fetch local Docker images for validation
    let docker_images = container::get_local_images(&docker_conn)
        .await
        .context("Failed to list local Docker images")?;

    let validated_nodes = validate::validate_and_resolve_node_versions(
        &manifest.nodes,
        &node_configs,
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
        let node_config = get_node_config(&node.model, &node_configs)
            .context(format!("Node config not found for model: {}", node.model))?;

        if !node_config.dedicated_management_interface {
            validate::check_mgmt_usage(&node.name, 0, &links_detailed, &bridges_detailed).context(
                format!(
                    "Management interface validation failed for node: {}",
                    node.name
                ),
            )?;
        }

        validate::check_interface_bounds(
            &node.name,
            &node_config.model,
            node_config.data_interface_count,
            node_config.reserved_interface_count,
            node_config.dedicated_management_interface,
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

    progress.send_status("Manifest validation complete".to_string())?;
    tracing::info!(lab_id = %lab_id, "Manifest validation completed successfully");
    phases_completed.push("ManifestValidation".to_string());

    // ========================================================================
    // PHASE 3: Database Records & Data Structure Building
    // ========================================================================
    progress.send_phase(
        data::UpPhase::DatabaseRecords,
        "Creating database records".to_string(),
    )?;

    tracing::info!(lab_id = %lab_id, lab_name = %manifest.name, "Creating database records");

    // Create lab record in database
    let lab_record = db::create_lab(&db, &manifest.name, lab_id, &db_user)
        .await
        .context("Failed to create lab record in database")?;
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
        let node_config = get_node_config(&node.model, &node_configs)?;

        tracing::info!(
            lab_id = %lab_id,
            node_name = %node.name,
            node_kind = ?node_config.kind,
            node_model = ?node_config.model,
            "Creating node database record"
        );

        // Build interface data structures
        let mut node_interfaces_detailed: Vec<data::InterfaceData> = vec![];
        let first_data_interface_idx = 1 + node_config.reserved_interface_count;
        let max_interface_idx = first_data_interface_idx + node_config.data_interface_count - 1;

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
            db::get_config_id(&node_config)?,
            lab_record_id.clone(),
        )
        .await?;

        lab_node_data.push(data::LabNodeData {
            name: node.name.clone(),
            model: node_config.model.clone(),
            kind: node_config.kind.clone(),
            index: node.index,
            record: lab_node,
        });

        match node_config.kind {
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

        let node_isolated_network = if matches!(node_config.kind, data::NodeKind::VirtualMachine) {
            Some(node_isolated_network_data(&node.name, node.index, lab_id))
        } else {
            None
        };

        let node_reserved_network = if matches!(node_config.kind, data::NodeKind::VirtualMachine)
            && node_config.reserved_interface_count > 0
        {
            Some(node_reserved_network_data(&node.name, node.index, lab_id))
        } else {
            None
        };

        if let Some(network) = node_isolated_network.clone() {
            progress.send_status(format!("Creating isolated network for node: {}", node.name))?;
            tracing::info!(
                lab_id = %lab_id,
                node_name = %node.name,
                network_type = "isolated",
                "Creating node isolated network"
            );
            let node_isolated_network = libvirt::IsolatedNetwork {
                network_name: network.network_name,
                bridge_name: network.bridge_name,
            };
            node_isolated_network.create(&qemu_conn)?;
        }

        if let Some(network) = node_reserved_network.clone() {
            progress.send_status(format!("Creating reserved network for node: {}", node.name))?;
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
    progress.send_phase(
        data::UpPhase::LabNetworkSetup,
        "Allocating lab network and creating management network".to_string(),
    )?;

    let lab_net = util::get_free_subnet(&config.management_prefix_ipv4.to_string())?;
    let gateway_ip = util::get_ipv4_addr(&lab_net, 1)?;
    let lab_router_ip = util::get_ipv4_addr(&lab_net, 2)?;

    tracing::info!(
        lab_id = %lab_id,
        subnet = %lab_net,
        gateway = %gateway_ip,
        boot_server = %lab_router_ip,
        "Allocated lab network subnet"
    );

    let lab_info = data::LabInfo {
        id: lab_id.to_string(),
        user: current_user.clone(),
        name: manifest.name.clone(),
        ipv4_network: lab_net,
        ipv4_gateway: gateway_ip,
        ipv4_router: lab_router_ip,
    };

    util::create_dir(&lab_dir)?;
    util::create_file(&format!("{lab_dir}/{LAB_FILE_NAME}"), lab_info.to_string())?;

    let mgmt_net = data::SherpaNetwork {
        v4: data::NetworkV4 {
            prefix: lab_net,
            first: gateway_ip,
            last: lab_net.broadcast(),
            boot_server: lab_router_ip,
            network: lab_net.network(),
            subnet_mask: lab_net.netmask(),
            hostmask: lab_net.hostmask(),
            prefix_length: lab_net.prefix_len(),
        },
    };
    let dns = util::default_dns(&lab_net)?;

    progress.send_status(format!(
        "Creating management network: {SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"
    ))?;

    tracing::info!(
        lab_id = %lab_id,
        network = %format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
        "Creating management network"
    );

    // Libvirt management network
    let management_network_obj = libvirt::NatNetwork {
        network_name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
        bridge_name: format!("{SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX}-{lab_id}"),
        ipv4_address: gateway_ip,
        ipv4_netmask: lab_net.netmask(),
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
    progress.send_phase(
        data::UpPhase::LinkCreation,
        format!("Creating {} point-to-point links", links_detailed.len()),
    )?;

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

        progress.send_status(format!(
            "Creating link #{} - {}::{} <-> {}::{}",
            idx, link.node_a, link.int_a, link.node_b, link.int_b
        ))?;

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
    progress.send_phase(
        data::UpPhase::ContainerNetworks,
        "Creating Docker networks for container links".to_string(),
    )?;

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

    tracing::info!(
        lab_id = %lab_id,
        networks_created = docker_net_count,
        "Docker macvlan networks created"
    );

    phases_completed.push("ContainerNetworks".to_string());

    // ========================================================================
    // PHASE 7: Shared Bridge Creation
    // ========================================================================
    progress.send_phase(
        data::UpPhase::SharedBridges,
        format!("Creating {} shared bridges", bridges_detailed.len()),
    )?;

    tracing::info!(
        lab_id = %lab_id,
        bridge_count = bridges_detailed.len(),
        "Creating shared bridges"
    );

    for bridge in bridges_detailed.iter() {
        let mut bridge_nodes = vec![];

        progress.send_status(format!(
            "Creating shared bridge #{} - {} ({} connections)",
            bridge.index,
            bridge.manifest_name,
            bridge.links.len()
        ))?;

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
    progress.send_phase(
        data::UpPhase::ZtpGeneration,
        "Generating ZTP configurations".to_string(),
    )?;

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

    // Container nodes ZTP generation
    for node in &mut container_nodes {
        let node_data = get_node_data(&node.name, &node_setup_data)?;
        let node_idx = node_data.index;
        let node_ip_idx = 10 + node_idx as u32;

        progress.send_status(format!("Creating container config: {}", node.name))?;

        tracing::info!(
            lab_id = %lab_id,
            node_name = %node.name,
            node_model = ?node.model,
            "Generating container configuration"
        );

        let dir = format!("{}/{}", lab_dir, node.name);
        let node_ipv4_address = util::get_ipv4_addr(&mgmt_net.v4.prefix, node_ip_idx)?;
        node.ipv4_address = Some(node_ipv4_address);

        // Persist management IPv4 to the database
        if let Some(node_data) = lab_node_data.iter().find(|n| n.name == node.name) {
            let record_id = db::get_node_id(&node_data.record)?;
            db::update_node_mgmt_ipv4(&db, record_id, &node_ipv4_address.to_string()).await?;
        }

        let node_config = get_node_config(&node.model, &node_configs)?;

        // Add to ZTP records for SSH config and DNS resolution
        ztp_records.push(data::ZtpRecord {
            node_name: node.name.clone(),
            config_file: format!("{}.conf", &node.name),
            ipv4_address: node_ipv4_address,
            mac_address: String::new(),
            ztp_method: node_config.ztp_method.clone(),
            ssh_port: SSH_PORT,
        });

        match node.model {
            data::NodeModel::AristaCeos => {
                let arista_template = template::AristaCeosZtpTemplate {
                    hostname: node.name.clone(),
                    user: sherpa_user.clone(),
                    dns: dns.clone(),
                    mgmt_ipv4_address: node.ipv4_address,
                    mgmt_ipv4: mgmt_net.v4.clone(),
                };
                let rendered_template = arista_template.render()?;
                let ztp_config = format!("{dir}/{}.conf", node.name);
                let ztp_volume = topology::VolumeMount {
                    src: ztp_config.clone(),
                    dst: ARISTA_CEOS_ZTP_VOLUME_MOUNT.to_string(),
                };
                util::create_dir(&dir)?;
                util::create_file(&ztp_config, rendered_template)?;

                node.image = Some(CONTAINER_ARISTA_CEOS_REPO.to_string());
                node.privileged = Some(true);
                node.environment_variables = Some(
                    CONTAINER_ARISTA_CEOS_ENV_VARS
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                );
                node.volumes = Some(vec![ztp_volume]);
                node.commands = Some(
                    CONTAINER_ARISTA_CEOS_COMMANDS
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                );
            }
            data::NodeModel::NokiaSrlinux => {
                node.image = Some(CONTAINER_NOKIA_SRLINUX_REPO.to_string());
                node.privileged = Some(true);
                node.environment_variables = Some(
                    CONTAINER_NOKIA_SRLINUX_ENV_VARS
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                );
                node.commands = Some(
                    CONTAINER_NOKIA_SRLINUX_COMMANDS
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                );
            }
            data::NodeModel::SurrealDb => {
                node.image = Some(CONTAINER_SURREAL_DB_REPO.to_string());
                node.commands = Some(
                    CONTAINER_SURREAL_DB_COMMANDS
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                );
            }
            _ => {}
        }
    }

    // VM nodes ZTP generation, disk setup, and domain template building
    tracing::info!(
        lab_id = %lab_id,
        vm_count = vm_nodes.len(),
        "Generating VM ZTP configurations and domain templates"
    );

    for node in &mut vm_nodes {
        let node_data = get_node_data(&node.name, &node_setup_data)?;
        let node_idx = node_data.index;
        let node_ip_idx = 10 + node_idx as u32;
        let node_name_with_lab = format!("{}-{}", node.name, lab_id);

        let node_config = get_node_config(&node.model, &node_configs)?;
        let mut disks: Vec<data::NodeDisk> = vec![];
        let hdd_bus = node_config.hdd_bus.clone();
        let cdrom_bus = node_config.cdrom_bus.clone();

        // Generate MAC address for management interface
        let mac_address = util::random_mac(KVM_OUI);
        let node_ipv4_address = util::get_ipv4_addr(&mgmt_net.v4.prefix, node_ip_idx)?;
        node.ipv4_address = Some(node_ipv4_address);

        // Persist management IPv4 to the database
        if let Some(node_data) = lab_node_data.iter().find(|n| n.name == node.name) {
            let record_id = db::get_node_id(&node_data.record)?;
            db::update_node_mgmt_ipv4(&db, record_id, &node_ipv4_address.to_string()).await?;
        }

        // Add to ZTP records
        ztp_records.push(data::ZtpRecord {
            node_name: node.name.clone(),
            config_file: format!("{}.conf", &node.name),
            ipv4_address: node_ipv4_address,
            mac_address: mac_address.to_string(),
            ztp_method: node_config.ztp_method.clone(),
            ssh_port: SSH_PORT,
        });

        // Build VM boot disk clone info
        let src_boot_disk = format!(
            "{}/{}/{}/virtioa.qcow2",
            config.images_dir, node_config.model, node_config.version
        );
        let dst_boot_disk = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-hdd.qcow2");

        clone_disks.push(data::CloneDisk {
            src: src_boot_disk.clone(),
            dst: dst_boot_disk.clone(),
        });

        // Handle CDROM ISO (e.g., aboot.iso for Arista vEOS)
        let (mut src_cdrom_iso, mut dst_cdrom_iso) = match &node_config.cdrom {
            Some(src_iso) => {
                let src = format!(
                    "{}/{}/{}/{}",
                    config.images_dir, node_config.model, node_config.version, src_iso
                );
                let dst = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}.iso");
                (Some(src), Some(dst))
            }
            None => (None, None),
        };

        // Handle config disk for Disk ZTP method (e.g., IOSv config disk)
        let (mut src_config_disk, mut dst_config_disk): (Option<String>, Option<String>) =
            (None, None);

        // Handle USB disk for USB ZTP method (e.g., Cumulus Linux, Juniper vEvolved)
        let (mut src_usb_disk, mut dst_usb_disk): (Option<String>, Option<String>) = (None, None);

        // Handle ignition config for Ignition ZTP method (e.g., Flatcar Linux)
        let (mut src_ignition_disk, mut dst_ignition_disk): (Option<String>, Option<String>) =
            (None, None);

        if node_config.ztp_enable {
            tracing::info!(
                lab_id = %lab_id,
                node_name = %node.name,
                model = ?node.model,
                ztp_method = ?node_config.ztp_method,
                ipv4 = %node_ipv4_address,
                "Generating VM ZTP configuration"
            );

            match node_config.ztp_method {
                data::ZtpMethod::CloudInit => {
                    progress
                        .send_status(format!("Creating Cloud-Init config for VM: {}", node.name))?;

                    let dir = format!("{lab_dir}/{}", node.name);
                    let mut cloud_init_user = template::CloudInitUser::sherpa()?;

                    match node.model {
                        data::NodeModel::CentosLinux
                        | data::NodeModel::AlmaLinux
                        | data::NodeModel::RockyLinux
                        | data::NodeModel::FedoraLinux
                        | data::NodeModel::OpensuseLinux
                        | data::NodeModel::RedhatLinux
                        | data::NodeModel::SuseLinux
                        | data::NodeModel::UbuntuLinux
                        | data::NodeModel::FreeBsd
                        | data::NodeModel::OpenBsd => {
                            let (admin_group, shell) = match node_config.os_variant {
                                data::OsVariant::Bsd => {
                                    ("wheel".to_string(), "/bin/sh".to_string())
                                }
                                _ => ("sudo".to_string(), "/bin/bash".to_string()),
                            };
                            cloud_init_user.groups = vec![admin_group];
                            cloud_init_user.shell = shell;

                            let cloud_init_config = template::CloudInitConfig {
                                hostname: node.name.clone(),
                                fqdn: format!("{}.{}", node.name.clone(), SHERPA_DOMAIN_NAME),
                                manage_etc_hosts: true,
                                ssh_pwauth: true,
                                users: vec![cloud_init_user],
                                ..Default::default()
                            };
                            let user_data_config = cloud_init_config.to_string()?;

                            let user_data = format!("{dir}/{CLOUD_INIT_USER_DATA}");
                            let meta_data = format!("{dir}/{CLOUD_INIT_META_DATA}");
                            let network_config = format!("{dir}/{CLOUD_INIT_NETWORK_CONFIG}");

                            util::create_dir(&dir)?;
                            util::create_file(&user_data, user_data_config)?;
                            util::create_file(&meta_data, "".to_string())?;

                            let ztp_interface = template::CloudInitNetwork::ztp_interface(
                                node_ipv4_address,
                                mac_address.clone(),
                                mgmt_net.v4.clone(),
                            );
                            let cloud_network_config = ztp_interface.to_string()?;
                            util::create_file(&network_config, cloud_network_config)?;

                            util::create_ztp_iso(&format!("{}/{}", dir, ZTP_ISO), dir)?;
                        }

                        data::NodeModel::AlpineLinux => {
                            let meta_data = template::MetaDataConfig {
                                instance_id: format!("iid-{}", node.name.clone()),
                                local_hostname: format!(
                                    "{}.{}",
                                    node.name.clone(),
                                    SHERPA_DOMAIN_NAME
                                ),
                            };
                            cloud_init_user.shell = "/bin/sh".to_string();
                            cloud_init_user.groups = vec!["wheel".to_string()];
                            let cloud_init_config = template::CloudInitConfig {
                                hostname: node.name.clone(),
                                fqdn: format!("{}.{}", node.name.clone(), SHERPA_DOMAIN_NAME),
                                manage_etc_hosts: true,
                                ssh_pwauth: true,
                                users: vec![cloud_init_user],
                                ..Default::default()
                            };
                            let meta_data_config = meta_data.to_string()?;
                            let user_data_config = cloud_init_config.to_string()?;

                            let user_data = format!("{dir}/{CLOUD_INIT_USER_DATA}");
                            let meta_data = format!("{dir}/{CLOUD_INIT_META_DATA}");
                            let network_config = format!("{dir}/{CLOUD_INIT_NETWORK_CONFIG}");

                            util::create_dir(&dir)?;
                            util::create_file(&user_data, user_data_config)?;
                            util::create_file(&meta_data, meta_data_config)?;

                            let ztp_interface = template::CloudInitNetwork::ztp_interface(
                                node_ipv4_address,
                                mac_address.clone(),
                                mgmt_net.v4.clone(),
                            );
                            let cloud_network_config = ztp_interface.to_string()?;
                            util::create_file(&network_config, cloud_network_config)?;

                            util::create_ztp_iso(&format!("{}/{}", dir, ZTP_ISO), dir)?;
                        }
                        _ => {
                            bail!(
                                "Cloud-Init ZTP method not supported for {}",
                                node_config.model
                            );
                        }
                    }
                    src_cdrom_iso = Some(format!("{lab_dir}/{}/{ZTP_ISO}", node.name));
                    dst_cdrom_iso = Some(format!(
                        "{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}.iso"
                    ));
                    tracing::debug!(
                        lab_id = %lab_id,
                        node_name = %node.name,
                        iso_path = %format!("{lab_dir}/{}/{ZTP_ISO}", node.name),
                        "Created CloudInit ISO"
                    );
                }
                data::ZtpMethod::Tftp => {
                    progress
                        .send_status(format!("Creating TFTP ZTP config for VM: {}", node.name))?;

                    match node.model {
                        data::NodeModel::AristaVeos => {
                            let arista_template = template::AristaVeosZtpTemplate {
                                hostname: node.name.clone(),
                                user: sherpa_user.clone(),
                                dns: dns.clone(),
                                mgmt_ipv4_address: Some(node_ipv4_address),
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = arista_template.render()?;
                            let ztp_config = format!("{tftp_dir}/{}.conf", node.name);
                            util::create_file(&ztp_config, rendered_template)?;
                        }
                        data::NodeModel::ArubaAoscx => {
                            let aruba_template = template::ArubaAoscxTemplate {
                                hostname: node.name.clone(),
                                user: sherpa_user.clone(),
                                dns: dns.clone(),
                                mgmt_ipv4_address: Some(node_ipv4_address),
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = aruba_template.render()?;
                            let ztp_config = format!("{tftp_dir}/{}.conf", node.name);
                            util::create_file(&ztp_config, rendered_template)?;
                        }
                        data::NodeModel::JuniperVevolved => {
                            let juniper_template = template::JunipervJunosZtpTemplate {
                                hostname: node.name.clone(),
                                user: sherpa_user.clone(),
                                mgmt_interface: node_config.management_interface.to_string(),
                                mgmt_ipv4_address: Some(node_ipv4_address),
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let juniper_rendered_template = juniper_template.render()?;
                            let ztp_config = format!("{tftp_dir}/{}.conf", node.name);
                            util::create_file(&ztp_config, juniper_rendered_template)?;
                        }
                        _ => {
                            bail!("TFTP ZTP method not supported for {}", node_config.model);
                        }
                    }
                    tracing::debug!(
                        lab_id = %lab_id,
                        node_name = %node.name,
                        config_path = %format!("{tftp_dir}/{}.conf", node.name),
                        "Created TFTP ZTP configuration"
                    );
                }
                data::ZtpMethod::Cdrom => {
                    progress
                        .send_status(format!("Creating CDROM ZTP config for VM: {}", node.name))?;

                    let dir = format!("{lab_dir}/{}", node.name);
                    let mut user = sherpa_user.clone();

                    match node.model {
                        data::NodeModel::CiscoCsr1000v
                        | data::NodeModel::CiscoCat8000v
                        | data::NodeModel::CiscoCat9000v => {
                            let license_boot_command =
                                if node.model == data::NodeModel::CiscoCat8000v {
                                    Some(
                                        "license boot level network-premier addon dna-premier"
                                            .to_string(),
                                    )
                                } else if node.model == data::NodeModel::CiscoCat9000v {
                                    Some(
                                        "license boot level network-advantage addon dna-advantage"
                                            .to_string(),
                                    )
                                } else {
                                    None
                                };

                            let key_hash = util::pub_ssh_key_to_md5_hash(&user.ssh_public_key.key)?;
                            user.ssh_public_key.key = key_hash;
                            let t = template::CiscoIosXeZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                mgmt_interface: node_config.management_interface.to_string(),
                                dns: dns.clone(),
                                license_boot_command,
                                mgmt_ipv4_address: Some(node_ipv4_address),
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let c = CISCO_IOSXE_ZTP_CONFIG.replace("-", "_");
                            let ztp_config = format!("{dir}/{c}");
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;
                            util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?;
                        }
                        data::NodeModel::CiscoAsav => {
                            let key_hash =
                                util::pub_ssh_key_to_sha256_hash(&user.ssh_public_key.key)?;
                            user.ssh_public_key.key = key_hash;
                            let t = template::CiscoAsavZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                dns: dns.clone(),
                                mgmt_ipv4_address: Some(node_ipv4_address),
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{CISCO_ASAV_ZTP_CONFIG}");
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;
                            util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?;
                        }
                        data::NodeModel::CiscoNexus9300v => {
                            let t = template::CiscoNxosZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                dns: dns.clone(),
                                mgmt_ipv4_address: Some(node_ipv4_address),
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{CISCO_NXOS_ZTP_CONFIG}");
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;
                            util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?;
                        }
                        data::NodeModel::CiscoIosxrv9000 => {
                            let t = template::CiscoIosxrZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                dns: dns.clone(),
                                mgmt_ipv4_address: Some(node_ipv4_address),
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{CISCO_IOSXR_ZTP_CONFIG}");
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;
                            util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?;
                        }
                        data::NodeModel::CiscoFtdv => {
                            let t = template::CiscoFtdvZtpTemplate {
                                eula: "accept".to_string(),
                                hostname: node.name.clone(),
                                admin_password: SHERPA_PASSWORD.to_string(),
                                dns1: Some(mgmt_net.v4.boot_server),
                                ipv4_mode: Some(template::CiscoFxosIpMode::Manual),
                                ipv4_addr: Some(node_ipv4_address),
                                ipv4_gw: Some(mgmt_net.v4.first),
                                ipv4_mask: Some(mgmt_net.v4.subnet_mask),
                                manage_locally: true,
                                ..Default::default()
                            };
                            let rendered_template = serde_json::to_string(&t)?;
                            let ztp_config = format!("{dir}/{CISCO_FTDV_ZTP_CONFIG}");
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;
                            util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?;
                        }
                        data::NodeModel::JuniperVsrxv3
                        | data::NodeModel::JuniperVrouter
                        | data::NodeModel::JuniperVswitch => {
                            let t = template::JunipervJunosZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                mgmt_interface: node_config.management_interface.to_string(),
                                mgmt_ipv4_address: Some(node_ipv4_address),
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{JUNIPER_ZTP_CONFIG}");
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;
                            util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?;
                        }
                        _ => {
                            bail!("CDROM ZTP method not supported for {}", node_config.model);
                        }
                    }
                    src_cdrom_iso = Some(format!("{lab_dir}/{}/{ZTP_ISO}", node.name));
                    dst_cdrom_iso = Some(format!(
                        "{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}.iso"
                    ));
                    tracing::debug!(
                        lab_id = %lab_id,
                        node_name = %node.name,
                        iso_path = %format!("{lab_dir}/{}/{ZTP_ISO}", node.name),
                        "Created CDROM ZTP ISO"
                    );
                }
                data::ZtpMethod::Disk => {
                    progress
                        .send_status(format!("Creating Disk ZTP config for VM: {}", node.name))?;

                    let dir = format!("{lab_dir}/{}", node.name);
                    let mut user = sherpa_user.clone();

                    match node.model {
                        data::NodeModel::CiscoIosv => {
                            let key_hash = util::pub_ssh_key_to_md5_hash(&user.ssh_public_key.key)?;
                            user.ssh_public_key.key = key_hash;
                            let t = template::CiscoIosvZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                mgmt_interface: node_config.management_interface.to_string(),
                                dns: dns.clone(),
                                mgmt_ipv4_address: Some(node_ipv4_address),
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let c = CISCO_IOSV_ZTP_CONFIG;
                            let ztp_config = format!("{dir}/{c}");
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;

                            // Clone blank disk image
                            let src_disk = format!(
                                "{}/{}/{}",
                                config.images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_IOSV
                            );
                            let dst_disk = format!("{dir}/{}-cfg.img", node.name);

                            // Copy blank disk and inject config
                            util::copy_file(&src_disk, &dst_disk)?;
                            util::copy_to_dos_image(&ztp_config, &dst_disk, "/")?;

                            src_config_disk = Some(dst_disk.clone());
                            dst_config_disk = Some(format!(
                                "{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-cfg.img"
                            ));
                        }
                        data::NodeModel::CiscoIosvl2 => {
                            let key_hash = util::pub_ssh_key_to_md5_hash(&user.ssh_public_key.key)?;
                            user.ssh_public_key.key = key_hash;
                            let t = template::CiscoIosvl2ZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                mgmt_interface: node_config.management_interface.to_string(),
                                dns: dns.clone(),
                                mgmt_ipv4_address: Some(node_ipv4_address),
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let c = CISCO_IOSV_ZTP_CONFIG;
                            let ztp_config = format!("{dir}/{c}");
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;

                            // Clone blank disk image
                            let src_disk = format!(
                                "{}/{}/{}",
                                config.images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_IOSV
                            );
                            let dst_disk = format!("{dir}/{}-cfg.img", node.name);

                            // Copy blank disk and inject config
                            util::copy_file(&src_disk, &dst_disk)?;
                            util::copy_to_dos_image(&ztp_config, &dst_disk, "/")?;

                            src_config_disk = Some(dst_disk.clone());
                            dst_config_disk = Some(format!(
                                "{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-cfg.img"
                            ));
                        }
                        data::NodeModel::CiscoIse => {
                            let t = template::CiscoIseZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                dns: dns.clone(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{CISCO_ISE_ZTP_CONFIG}");
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;

                            // Clone blank disk image
                            let src_disk = format!(
                                "{}/{}/{}",
                                config.images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_ISE
                            );
                            let dst_disk = format!("{dir}/{node_name_with_lab}-cfg.img");

                            // Copy blank disk and inject config
                            util::copy_file(&src_disk, &dst_disk)?;
                            util::copy_to_ext4_image(vec![&ztp_config], &dst_disk, "/")?;

                            src_config_disk = Some(dst_disk.clone());
                            dst_config_disk = Some(format!(
                                "{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-cfg.img"
                            ));
                        }
                        _ => {
                            bail!("Disk ZTP method not supported for {}", node_config.model);
                        }
                    }
                    tracing::debug!(
                        lab_id = %lab_id,
                        node_name = %node.name,
                        "Created Disk ZTP configuration"
                    );
                }
                data::ZtpMethod::Usb => {
                    progress
                        .send_status(format!("Creating USB ZTP config for VM: {}", node.name))?;

                    let dir = format!("{lab_dir}/{}", node.name);
                    let user = sherpa_user.clone();

                    match node_config.model {
                        data::NodeModel::CumulusLinux => {
                            let t = template::CumulusLinuxZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                dns: dns.clone(),
                                mgmt_ipv4_address: Some(node_ipv4_address),
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{CUMULUS_ZTP}");
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;

                            // Clone USB disk image
                            let src_usb = format!(
                                "{}/{}/{}",
                                config.images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_FAT32
                            );
                            let dst_usb = format!("{dir}/cfg.img");

                            // Copy blank USB and inject config
                            util::copy_file(&src_usb, &dst_usb)?;
                            util::copy_to_dos_image(&ztp_config, &dst_usb, "/")?;

                            src_usb_disk = Some(dst_usb.clone());
                            dst_usb_disk = Some(format!(
                                "{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-cfg.img"
                            ));
                        }
                        data::NodeModel::JuniperVevolved => {
                            let t = template::JunipervJunosZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                mgmt_interface: node_config.management_interface.to_string(),
                                mgmt_ipv4_address: Some(node_ipv4_address),
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{JUNIPER_ZTP_CONFIG}");
                            let ztp_config_tgz = format!("{dir}/{JUNIPER_ZTP_CONFIG_TGZ}");

                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;

                            // Clone USB disk image
                            let src_usb = format!(
                                "{}/{}/{}",
                                config.images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_JUNOS
                            );
                            let dst_usb = format!("{dir}/cfg.img");

                            // Copy blank USB
                            util::copy_file(&src_usb, &dst_usb)?;

                            // Create tar.gz config file
                            util::create_config_archive(&ztp_config, &ztp_config_tgz)?;

                            // Copy archive to USB disk
                            util::copy_to_dos_image(&ztp_config_tgz, &dst_usb, "/")?;

                            src_usb_disk = Some(dst_usb.clone());
                            dst_usb_disk = Some(format!(
                                "{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-cfg.img"
                            ));
                        }
                        _ => {
                            bail!("USB ZTP method not supported for {}", node_config.model);
                        }
                    }
                    tracing::debug!(
                        lab_id = %lab_id,
                        node_name = %node.name,
                        "Created USB ZTP configuration"
                    );
                }
                data::ZtpMethod::Http => {
                    progress
                        .send_status(format!("Creating HTTP ZTP config for VM: {}", node.name))?;

                    let dir = format!("{lab_dir}/{ZTP_DIR}/{NODE_CONFIGS_DIR}");

                    match node_config.model {
                        data::NodeModel::SonicLinux => {
                            let sonic_ztp_file_map = template::SonicLinuxZtp::file_map(
                                &node.name,
                                &mgmt_net.v4.boot_server,
                            );

                            let ztp_init = format!("{dir}/{}.conf", &node.name);
                            let sonic_ztp = template::SonicLinuxZtp {
                                hostname: node.name.clone(),
                                mgmt_ipv4: mgmt_net.v4.clone(),
                                mgmt_ipv4_address: Some(node_ipv4_address),
                            };
                            let ztp_config = format!("{dir}/{}_config_db.json", &node.name);
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_init, sonic_ztp_file_map)?;
                            util::create_file(&ztp_config, sonic_ztp.config())?;
                        }
                        _ => {
                            bail!("HTTP ZTP method not supported for {}", node_config.model);
                        }
                    }
                    tracing::debug!(
                        lab_id = %lab_id,
                        node_name = %node.name,
                        config_dir = %format!("{lab_dir}/{ZTP_DIR}/{NODE_CONFIGS_DIR}"),
                        "Created HTTP ZTP configuration"
                    );
                }
                data::ZtpMethod::Ignition => {
                    progress
                        .send_status(format!("Creating Ignition config for VM: {}", node.name))?;

                    let user = sherpa_user.clone();
                    let dir = format!("{lab_dir}/{}", node.name);
                    let dev_name = node.name.clone();

                    // Build authorized keys list
                    let mut authorized_keys = vec![format!(
                        "{} {} {}",
                        user.ssh_public_key.algorithm,
                        user.ssh_public_key.key,
                        user.ssh_public_key.comment.clone().unwrap_or("".to_owned())
                    )];

                    let manifest_authorized_keys: Vec<String> =
                        node.ssh_authorized_keys.clone().unwrap_or(vec![]);

                    let manifest_authorized_key_files: Vec<String> = node
                        .ssh_authorized_key_files
                        .iter()
                        .flatten()
                        .map(|file| -> Result<String> {
                            let ssh_key = util::get_ssh_public_key(&file.source)?;
                            Ok(format!(
                                "{} {} {}",
                                ssh_key.algorithm,
                                ssh_key.key,
                                ssh_key.comment.unwrap_or("".to_owned())
                            ))
                        })
                        .collect::<Result<Vec<String>>>()?;

                    authorized_keys.extend(manifest_authorized_keys);
                    authorized_keys.extend(manifest_authorized_key_files);

                    let ignition_user = template::IgnitionUser {
                        name: user.username.clone(),
                        password_hash: SHERPA_PASSWORD_HASH.to_owned(),
                        ssh_authorized_keys: authorized_keys,
                        groups: vec!["wheel".to_owned(), "docker".to_owned()],
                    };

                    let hostname_file = template::IgnitionFile {
                        path: "/etc/hostname".to_owned(),
                        mode: 644,
                        contents: template::IgnitionFileContents::new(&format!("data:,{dev_name}")),
                        ..Default::default()
                    };

                    let disable_update = template::IgnitionFile::disable_updates();
                    let sudo_config_base64 =
                        util::base64_encode(&format!("{SHERPA_USERNAME} ALL=(ALL) NOPASSWD: ALL"));
                    let sudo_config_file = template::IgnitionFile {
                        path: format!("/etc/sudoers.d/{SHERPA_USERNAME}"),
                        mode: 440,
                        contents: template::IgnitionFileContents::new(&format!(
                            "data:;base64,{sudo_config_base64}"
                        )),
                        ..Default::default()
                    };

                    let manifest_text_files: Vec<template::IgnitionFile> = node
                        .text_files
                        .iter()
                        .flatten()
                        .map(|file| {
                            let encoded_file = util::base64_encode_file(&file.source)?;

                            Ok(template::IgnitionFile {
                                path: file.destination.clone(),
                                mode: file.permissions,
                                overwrite: None,
                                contents: template::IgnitionFileContents::new(&format!(
                                    "data:;base64,{encoded_file}"
                                )),
                                user: Some(template::IgnitionFileParams {
                                    name: file.user.clone(),
                                }),
                                group: Some(template::IgnitionFileParams {
                                    name: file.group.clone(),
                                }),
                            })
                        })
                        .collect::<Result<Vec<template::IgnitionFile>>>()?;

                    let manifest_binary_disk_files = node.binary_files.clone().unwrap_or(vec![]);

                    let manifest_systemd_units: Vec<template::IgnitionUnit> = node
                        .systemd_units
                        .iter()
                        .flatten()
                        .map(|file| {
                            let file_contents = util::load_file(file.source.as_str())?;
                            Ok(template::IgnitionUnit {
                                name: file.name.clone(),
                                enabled: Some(file.enabled),
                                contents: Some(file_contents),
                                ..Default::default()
                            })
                        })
                        .collect::<Result<Vec<template::IgnitionUnit>>>()?;

                    match node_config.model {
                        data::NodeModel::FlatcarLinux => {
                            let mut units = vec![];
                            units.push(template::IgnitionUnit::mount_container_disk());
                            units.extend(manifest_systemd_units);

                            let container_disk = template::IgnitionFileSystem::default();

                            let mut files = vec![sudo_config_file, hostname_file, disable_update];
                            files.extend(manifest_text_files);

                            // Always add interface config (node_ipv4_address is always present in server)
                            files.push(template::IgnitionFile::ztp_interface(
                                node_ipv4_address,
                                mgmt_net.v4.clone(),
                            )?);

                            let ignition_config = template::IgnitionConfig::new(
                                vec![ignition_user],
                                files,
                                vec![],
                                units,
                                vec![],
                                vec![container_disk],
                            );
                            let flatcar_config = ignition_config.to_json_pretty()?;
                            let src_ztp_file = format!("{dir}/{ZTP_JSON}");
                            let dst_ztp_file =
                                format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-cfg.ign");

                            util::create_dir(&dir)?;
                            util::create_file(&src_ztp_file, flatcar_config)?;

                            // Copy blank disk for container data
                            let src_data_disk = format!(
                                "{}/{}/{}",
                                config.images_dir,
                                SHERPA_BLANK_DISK_DIR,
                                SHERPA_BLANK_DISK_EXT4_500MB
                            );
                            let dst_disk =
                                format!("{dir}/{node_name_with_lab}-{CONTAINER_DISK_NAME}");

                            util::copy_file(&src_data_disk, &dst_disk)?;

                            let disk_files: Vec<&str> = manifest_binary_disk_files
                                .iter()
                                .map(|x| x.source.as_str())
                                .collect();

                            // Copy container images into the container disk
                            if !disk_files.is_empty() {
                                util::copy_to_ext4_image(disk_files, &dst_disk, "/")?;
                            }

                            src_config_disk = Some(dst_disk.to_owned());
                            dst_config_disk = Some(format!(
                                "{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-{CONTAINER_DISK_NAME}"
                            ));

                            src_ignition_disk = Some(src_ztp_file.to_owned());
                            dst_ignition_disk = Some(dst_ztp_file.to_owned());
                        }
                        _ => {
                            bail!(
                                "Ignition ZTP method not supported for {}",
                                node_config.model
                            );
                        }
                    }
                    tracing::debug!(
                        lab_id = %lab_id,
                        node_name = %node.name,
                        "Created Ignition ZTP configuration"
                    );
                }
                _ => {
                    // Other ZTP methods not yet implemented
                    progress.send_status(format!(
                        "ZTP method {:?} not yet implemented for VM: {}",
                        node_config.ztp_method, node.name
                    ))?;
                }
            }
        }

        // Clone CDROM ISO if present
        if let (Some(src_iso), Some(dst_iso)) = (src_cdrom_iso.clone(), dst_cdrom_iso.clone()) {
            clone_disks.push(data::CloneDisk {
                src: src_iso,
                dst: dst_iso.clone(),
            });
            disks.push(data::NodeDisk {
                disk_device: data::DiskDevices::Cdrom,
                driver_name: data::DiskDrivers::Qemu,
                driver_format: data::DiskFormats::Raw,
                src_file: dst_iso.clone(),
                target_dev: data::DiskTargets::target(&cdrom_bus, disks.len() as u8)?,
                target_bus: cdrom_bus.clone(),
            });
        }

        // Add boot disk (second position to match client order)
        disks.push(data::NodeDisk {
            disk_device: data::DiskDevices::File,
            driver_name: data::DiskDrivers::Qemu,
            driver_format: data::DiskFormats::Qcow2,
            src_file: dst_boot_disk.clone(),
            target_dev: data::DiskTargets::target(&hdd_bus, disks.len() as u8)?,
            target_bus: hdd_bus.clone(),
        });

        // Clone config disk if present (for Disk ZTP method)
        if let (Some(src_disk), Some(dst_disk)) = (src_config_disk.clone(), dst_config_disk.clone())
        {
            clone_disks.push(data::CloneDisk {
                src: src_disk,
                dst: dst_disk.clone(),
            });
            disks.push(data::NodeDisk {
                disk_device: data::DiskDevices::File,
                driver_name: data::DiskDrivers::Qemu,
                driver_format: data::DiskFormats::Raw,
                src_file: dst_disk.clone(),
                target_dev: data::DiskTargets::target(&hdd_bus, disks.len() as u8)?,
                target_bus: hdd_bus.clone(),
            });
        }

        // Clone USB disk if present (for USB ZTP method)
        if let (Some(src_disk), Some(dst_disk)) = (src_usb_disk.clone(), dst_usb_disk.clone()) {
            clone_disks.push(data::CloneDisk {
                src: src_disk,
                dst: dst_disk.clone(),
            });
            disks.push(data::NodeDisk {
                disk_device: data::DiskDevices::File,
                driver_name: data::DiskDrivers::Qemu,
                driver_format: data::DiskFormats::Raw,
                src_file: dst_disk.clone(),
                target_dev: data::DiskTargets::target(&hdd_bus, disks.len() as u8)?,
                target_bus: hdd_bus.clone(),
            });
        }

        // Clone ignition config if present (for Ignition ZTP method)
        if let (Some(src_ignition), Some(dst_ignition)) =
            (src_ignition_disk.clone(), dst_ignition_disk.clone())
        {
            clone_disks.push(data::CloneDisk {
                src: src_ignition,
                dst: dst_ignition.clone(),
            });
            // Note: Ignition config is passed as QEMU command line argument, not as disk
        }

        // Build interfaces list
        let mut interfaces: Vec<data::Interface> = vec![];
        for interface in node_data.interfaces.iter() {
            match &interface.data {
                data::NodeInterface::Management => {
                    interfaces.push(data::Interface {
                        name: util::dasher(&node_config.management_interface.to_string()),
                        num: interface.index,
                        mtu: node_config.interface_mtu,
                        mac_address: mac_address.to_string(),
                        connection_type: data::ConnectionTypes::Management,
                        interface_connection: None,
                    });
                }
                data::NodeInterface::Reserved => {
                    interfaces.push(data::Interface {
                        name: format!("int{}", interface.index),
                        num: interface.index,
                        mtu: node_config.interface_mtu,
                        mac_address: util::random_mac(KVM_OUI),
                        connection_type: data::ConnectionTypes::Reserved,
                        interface_connection: None,
                    });
                }
                data::NodeInterface::Bridge(bridge) => {
                    interfaces.push(data::Interface {
                        name: bridge.name.clone(),
                        num: interface.index,
                        mtu: node_config.interface_mtu,
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
                        local_loopback: util::get_ip(local_id).to_string(),
                        source_id: peer.peer_node_index,
                        source_port: util::id_to_port(source_id),
                        source_loopback: util::get_ip(source_id).to_string(),
                    };
                    interfaces.push(data::Interface {
                        name: bridge_name,
                        num: interface.index,
                        mtu: node_config.interface_mtu,
                        mac_address: util::random_mac(KVM_OUI),
                        connection_type: data::ConnectionTypes::PeerBridge,
                        interface_connection: Some(interface_connection),
                    });
                }
                data::NodeInterface::Disabled => {
                    // Disabled interfaces are added and connected to isolated network with link state down
                    interfaces.push(data::Interface {
                        name: util::dasher(&util::interface_from_idx(
                            &node.model,
                            interface.index,
                        )?),
                        num: interface.index,
                        mtu: node_config.interface_mtu,
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

        // Build QEMU commands if needed
        let qemu_commands = match node_config.model {
            data::NodeModel::JuniperVrouter => data::QemuCommand::juniper_vrouter(),
            data::NodeModel::JuniperVswitch => data::QemuCommand::juniper_vswitch(),
            data::NodeModel::JuniperVevolved => data::QemuCommand::juniper_vevolved(),
            data::NodeModel::FlatcarLinux => {
                if let Some(dst_ignition) = dst_ignition_disk.clone() {
                    data::QemuCommand::ignition_config(&dst_ignition)
                } else {
                    vec![]
                }
            }
            _ => vec![],
        };

        // Create DomainTemplate
        let domain = template::DomainTemplate {
            qemu_bin: config.qemu_bin.clone(),
            name: node_name_with_lab.clone(),
            memory: node.memory.unwrap_or(node_config.memory),
            cpu_architecture: node_config.cpu_architecture.clone(),
            cpu_model: node_config.cpu_model.clone(),
            machine_type: node_config.machine_type.clone(),
            cpu_count: node.cpu_count.unwrap_or(node_config.cpu_count),
            vmx_enabled: node_config.vmx_enabled,
            bios: node_config.bios.clone(),
            disks,
            interfaces,
            interface_type: node_config.interface_type.clone(),
            loopback_ipv4: util::get_ip(node_idx as u8).to_string(),
            telnet_port: TELNET_PORT,
            qemu_commands,
            lab_id: lab_id.to_string(),
            management_network,
            isolated_network: isolated_network.network_name,
            reserved_network,
        };
        domains.push(domain);
    }

    phases_completed.push("ZtpGeneration".to_string());

    // ========================================================================
    // PHASE 9: Sherpa Router & ZTP File Creation
    // ========================================================================
    progress.send_phase(
        data::UpPhase::BootContainers,
        "Creating boot containers and ZTP files".to_string(),
    )?;

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
    };

    tracing::info!(
        lab_id = %lab_id,
        container = %format!("{CONTAINER_DNSMASQ_NAME}-{lab_id}"),
        boot_server_ip = %boot_server_ipv4,
        "Starting dnsmasq boot container"
    );

    container::run_container(
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
    )
    .await?;

    phases_completed.push("Sherpa Router".to_string());

    // ========================================================================
    // PHASE 10: Disk Cloning (For VMs)
    // ========================================================================
    progress.send_phase(data::UpPhase::DiskCloning, "Cloning VM disks".to_string())?;

    if !clone_disks.is_empty() {
        let disk_timer = Instant::now();
        let disk_count = clone_disks.len();

        tracing::info!(
            lab_id = %lab_id,
            disk_count = disk_count,
            "Starting disk cloning (parallel)"
        );

        progress.send_status(format!("Cloning {} disks in parallel", disk_count))?;

        let qemu_conn_arc = Arc::clone(&qemu_conn);
        let lab_id_clone = lab_id.to_string();
        let tasks: Vec<_> = clone_disks
            .into_iter()
            .map(|disk| {
                let conn = Arc::clone(&qemu_conn_arc);
                let progress_clone = progress.clone();
                let src = disk.src.clone();
                let dst = disk.dst.clone();
                let lab_id_task = lab_id_clone.clone();

                tokio::task::spawn(async move {
                    // Extract node name from disk path (e.g., "router1-abc123-hdd.qcow2" -> "router1-abc123")
                    let node_name = dst
                        .split('/')
                        .last()
                        .and_then(|f| f.strip_suffix("-hdd.qcow2"))
                        .unwrap_or("unknown");

                    tracing::info!(
                        lab_id = %lab_id_task,
                        node_name = %node_name,
                        src = %src,
                        "Cloning disk"
                    );

                    progress_clone.send_status(format!("Cloning disk from: {}", src))?;

                    // libvirt operations are synchronous, so we need to use spawn_blocking
                    let conn_for_blocking = conn.clone();
                    let src_for_blocking = src.clone();
                    let dst_for_blocking = dst.clone();

                    tokio::task::spawn_blocking(move || -> Result<()> {
                        libvirt::clone_disk(
                            &conn_for_blocking,
                            &src_for_blocking,
                            &dst_for_blocking,
                        )
                        .with_context(|| {
                            format!(
                                "Failed to clone disk from: {} to: {}",
                                src_for_blocking, dst_for_blocking
                            )
                        })?;
                        Ok(())
                    })
                    .await
                    .map_err(|e| anyhow!("Task join error: {:?}", e))??;

                    tracing::info!(
                        lab_id = %lab_id_task,
                        node_name = %node_name,
                        dst = %dst,
                        "Disk cloned successfully"
                    );

                    progress_clone.send_status(format!("Cloned disk to: {}", dst))?;
                    Ok::<(), anyhow::Error>(())
                })
            })
            .collect();

        // Wait for all tasks to complete
        for task in tasks {
            task.await.context("Disk cloning task failed")??;
        }

        let elapsed = disk_timer.elapsed().as_secs();
        tracing::info!(
            lab_id = %lab_id,
            disk_count = disk_count,
            duration_secs = elapsed,
            "All disks cloned successfully"
        );
        progress.send_status("All disks cloned successfully".to_string())?;
    } else {
        tracing::info!(lab_id = %lab_id, "No disks to clone");
        progress.send_status("No disks to clone".to_string())?;
    }

    phases_completed.push("DiskCloning".to_string());

    // ========================================================================
    // PHASE 11: VM Creation
    // ========================================================================
    progress.send_phase(data::UpPhase::VmCreation, "Creating VMs".to_string())?;

    if !domains.is_empty() {
        let vm_timer = std::time::Instant::now();
        let vm_count = domains.len();

        tracing::info!(
            lab_id = %lab_id,
            vm_count = vm_count,
            "Starting VM creation in parallel"
        );
        progress.send_status(format!("Creating {} VMs in parallel", domains.len()))?;

        let qemu_conn_arc = Arc::clone(&qemu_conn);
        let lab_id_for_tasks = lab_id.clone();
        let tasks: Vec<_> = domains
            .into_iter()
            .map(|domain| {
                let conn = Arc::clone(&qemu_conn_arc);
                let progress_clone = progress.clone();
                let vm_name = domain.name.clone();
                let lab_id_clone = lab_id_for_tasks.clone();
                let memory_mb = domain.memory;
                let vcpus = domain.cpu_count;

                tokio::task::spawn(async move {
                    tracing::info!(
                        lab_id = %lab_id_clone,
                        vm_name = %vm_name,
                        memory_mb = memory_mb,
                        vcpus = vcpus,
                        "Creating VM"
                    );
                    progress_clone.send_status(format!("Creating VM: {}", vm_name))?;

                    // Render the XML template (synchronous operation)
                    let rendered_xml = domain
                        .render()
                        .with_context(|| format!("Failed to render XML for VM: {}", vm_name))?;

                    tracing::debug!(
                        lab_id = %lab_id_clone,
                        vm_name = %vm_name,
                        "Rendered domain XML"
                    );

                    // libvirt operations are synchronous, so we need to use spawn_blocking
                    let conn_for_blocking = conn.clone();
                    let vm_name_for_blocking = vm_name.clone();
                    let lab_id_for_blocking = lab_id_clone.clone();

                    tokio::task::spawn_blocking(move || -> Result<()> {
                        libvirt::create_vm(&conn_for_blocking, &rendered_xml).with_context(
                            || format!("Failed to create VM: {}", vm_name_for_blocking),
                        )?;
                        tracing::info!(
                            lab_id = %lab_id_for_blocking,
                            vm_name = %vm_name_for_blocking,
                            "VM created successfully"
                        );
                        Ok(())
                    })
                    .await
                    .map_err(|e| anyhow!("Task join error: {:?}", e))??;

                    progress_clone.send_status(format!("Created VM: {}", vm_name))?;
                    Ok::<(), anyhow::Error>(())
                })
            })
            .collect();

        // Wait for all tasks to complete
        for task in tasks {
            task.await.context("VM creation task failed")??;
        }

        let elapsed = vm_timer.elapsed().as_secs();
        tracing::info!(
            lab_id = %lab_id,
            vm_count = vm_count,
            duration_secs = elapsed,
            "All VMs created successfully"
        );
        progress.send_status("All VMs created successfully".to_string())?;
    } else {
        tracing::info!(lab_id = %lab_id, "No VMs to create");
        progress.send_status("No VMs to create".to_string())?;
    }

    phases_completed.push("VmCreation".to_string());

    // ========================================================================
    // PHASE 12: SSH Config & Network Map Building
    // ========================================================================
    progress.send_phase(
        data::UpPhase::SshConfig,
        "Generating SSH config".to_string(),
    )?;

    tracing::info!(lab_id = %lab_id, "Generating SSH configuration");

    // Load server config to get server_ipv4
    let config_file_path = format!("{SHERPA_BASE_DIR}/{SHERPA_CONFIG_DIR}/{SHERPA_CONFIG_FILE}");
    let config_contents =
        util::load_file(&config_file_path).context("Failed to load sherpa.toml config")?;
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
    progress.send_status("SSH config file created".to_string())?;

    // Read SSH private key for transfer to client
    let ssh_private_key_path = format!(
        "{}/{}/{}",
        SHERPA_BASE_DIR, SHERPA_SSH_DIR, SHERPA_SSH_PRIVATE_KEY_FILE
    );
    let ssh_private_key =
        util::load_file(&ssh_private_key_path).context("Failed to read SSH private key")?;
    tracing::debug!(
        lab_id = %lab_id,
        key_path = %ssh_private_key_path,
        "Loaded SSH private key"
    );
    progress.send_status("SSH private key loaded".to_string())?;

    // Build container network mappings
    let mut container_link_networks: HashMap<String, Vec<data::ContainerNetworkAttachment>> =
        HashMap::new();

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
            container_link_networks
                .entry(node_a_data.name.clone())
                .or_insert_with(Vec::new)
                .push(data::ContainerNetworkAttachment {
                    name: docker_net_name,
                    ipv4_address: None,
                });
        }

        if node_b_data.kind == data::NodeKind::Container {
            let docker_net_name =
                format!("{}-ethb{}-{}", node_b_data.name, link_data.index, lab_id);
            container_link_networks
                .entry(node_b_data.name.clone())
                .or_insert_with(Vec::new)
                .push(data::ContainerNetworkAttachment {
                    name: docker_net_name,
                    ipv4_address: None,
                });
        }
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
    progress.send_phase(
        data::UpPhase::NodeReadiness,
        format!(
            "Waiting for {} nodes to become ready (up to {} seconds)",
            container_nodes.len() + vm_nodes.len(),
            READINESS_TIMEOUT
        ),
    )?;

    let start_time_readiness = Instant::now();
    let readiness_timer = std::time::Instant::now();
    let timeout = Duration::from_secs(READINESS_TIMEOUT);
    let mut connected_nodes = std::collections::HashSet::new();
    let mut node_info_list = vec![];

    let all_lab_nodes = vec![
        container_nodes.clone(),
        unikernel_nodes.clone(),
        vm_nodes.clone(),
    ]
    .concat();
    let total_lab_nodes = all_lab_nodes.len();

    tracing::info!(
        lab_id = %lab_id,
        total_nodes = total_lab_nodes,
        containers = container_nodes.len(),
        vms = vm_nodes.len(),
        unikernels = unikernel_nodes.len(),
        timeout_secs = READINESS_TIMEOUT,
        "Starting node readiness polling"
    );

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

    progress.send_status(format!("Waiting for nodes: {}", node_names))?;

    while start_time_readiness.elapsed() < timeout && connected_nodes.len() < total_lab_nodes {
        // Start containers
        for container in &container_nodes {
            if connected_nodes.contains(&container.name) {
                continue;
            }

            let mgmt_ipv4 = container.ipv4_address.map(|i| i.to_string());
            let container_name = format!("{}-{}", container.name, lab_id);

            // Extract image and version with proper error handling
            let container_image_name = container
                .image
                .as_ref()
                .ok_or_else(|| anyhow!("Container image not set for node: {}", container.name))?;

            let container_version = container
                .version
                .as_ref()
                .ok_or_else(|| anyhow!("Container version not set for node: {}", container.name))?;

            let container_image = format!("{}:{}", container_image_name, container_version);
            let privileged = container.privileged.unwrap_or(false);
            let env_vars = container
                .environment_variables
                .clone()
                .unwrap_or_else(|| vec![]);
            let commands = container.commands.clone().unwrap_or_else(|| vec![]);
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
            };

            let mut additional_networks = vec![];
            if let Some(link_networks) = container_link_networks.get(&container.name) {
                additional_networks.extend_from_slice(link_networks);
            }

            tracing::info!(
                lab_id = %lab_id,
                node_name = %container.name,
                node_kind = "Container",
                image = %container_image,
                ipv4 = ?mgmt_ipv4,
                privileged = privileged,
                "Starting container"
            );

            tracing::debug!(
                lab_id = %lab_id,
                node_name = %container.name,
                additional_networks = additional_networks.len(),
                volumes = volumes.len(),
                env_vars = env_vars.len(),
                commands = commands.len(),
                "Container configuration"
            );

            container::run_container(
                &docker_conn,
                &container_name,
                &container_image,
                env_vars,
                volumes,
                vec![],
                management_network_attachment,
                additional_networks,
                commands,
                privileged,
            )
            .await?;

            tracing::info!(
                lab_id = %lab_id,
                node_name = %container.name,
                "Container started and ready"
            );
            progress.send_status(format!("Node {} - Started", container.name))?;
            connected_nodes.insert(container.name.clone());

            // Update node state in DB to Running
            if let Some(node_data) = lab_node_data.iter().find(|n| n.name == container.name) {
                let record_id = db::get_node_id(&node_data.record)?;
                db::update_node_state(&db, record_id, NodeState::Running).await?;
            }

            node_info_list.push(data::NodeInfo {
                name: container.name.clone(),
                kind: "Container".to_string(),
                model: container.model.clone(),
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
                match validate::tcp_connect(&vm_data.ipv4_address.to_string(), SSH_PORT)? {
                    true => {
                        tracing::info!(
                            lab_id = %lab_id,
                            node_name = %vm.name,
                            node_kind = "VirtualMachine",
                            ipv4 = %vm_data.ipv4_address,
                            "VM ready (SSH accessible)"
                        );
                        progress
                            .send_status(format!("Node {} - Ready (SSH accessible)", vm.name))?;
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
                            model: vm.model.clone(),
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
                        progress.send_status(format!("Node {} - Waiting for SSH", vm.name))?;
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
        progress.send_status("All nodes are ready!".to_string())?;
    } else {
        tracing::warn!(
            lab_id = %lab_id,
            nodes_ready = connected_nodes.len(),
            total_nodes = total_lab_nodes,
            duration_secs = readiness_elapsed,
            timeout_secs = READINESS_TIMEOUT,
            "Timeout reached - not all nodes ready"
        );
        progress.send_status(format!(
            "Timeout reached. {} of {} nodes are ready.",
            connected_nodes.len(),
            total_lab_nodes
        ))?;
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
}
