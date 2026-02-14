use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use askama::Template;

use super::boot_containers::{create_boot_containers, create_ztp_files};
use super::manifest_processing::{get_node_config, process_manifest_links, process_manifest_nodes};
use shared::data;
use shared::konst::{
    ARISTA_CEOS_ZTP_VOLUME_MOUNT, BRIDGE_PREFIX, CISCO_ASAV_ZTP_CONFIG, CISCO_FTDV_ZTP_CONFIG,
    CISCO_IOSV_ZTP_CONFIG, CISCO_IOSXE_ZTP_CONFIG, CISCO_IOSXR_ZTP_CONFIG, CISCO_ISE_ZTP_CONFIG,
    CISCO_NXOS_ZTP_CONFIG, CLOUD_INIT_META_DATA, CLOUD_INIT_NETWORK_CONFIG, CLOUD_INIT_USER_DATA,
    CONTAINER_ARISTA_CEOS_COMMANDS, CONTAINER_ARISTA_CEOS_ENV_VARS, CONTAINER_ARISTA_CEOS_REPO,
    CONTAINER_DISK_NAME, CONTAINER_NOKIA_SRLINUX_COMMANDS, CONTAINER_NOKIA_SRLINUX_ENV_VARS,
    CONTAINER_NOKIA_SRLINUX_REPO, CONTAINER_SURREAL_DB_COMMANDS, CONTAINER_SURREAL_DB_REPO,
    CUMULUS_ZTP, JUNIPER_ZTP_CONFIG, JUNIPER_ZTP_CONFIG_TGZ, KVM_OUI, LAB_FILE_NAME,
    NODE_CONFIGS_DIR, READINESS_SLEEP, READINESS_TIMEOUT, SHERPA_BASE_DIR, SHERPA_BLANK_DISK_DIR,
    SHERPA_BLANK_DISK_EXT4_500MB, SHERPA_BLANK_DISK_FAT32, SHERPA_BLANK_DISK_IOSV,
    SHERPA_BLANK_DISK_ISE, SHERPA_BLANK_DISK_JUNOS, SHERPA_DB_NAME, SHERPA_DB_NAMESPACE,
    SHERPA_DB_PORT, SHERPA_DB_SERVER, SHERPA_DOMAIN_NAME, SHERPA_ISOLATED_NETWORK_BRIDGE_PREFIX,
    SHERPA_ISOLATED_NETWORK_NAME, SHERPA_LABS_DIR, SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX,
    SHERPA_MANAGEMENT_NETWORK_NAME, SHERPA_PASSWORD, SHERPA_PASSWORD_HASH,
    SHERPA_RESERVED_NETWORK_BRIDGE_PREFIX, SHERPA_RESERVED_NETWORK_NAME, SHERPA_SSH_CONFIG_FILE,
    SHERPA_STORAGE_POOL_PATH, SHERPA_USERNAME, SSH_PORT, TELNET_PORT, TFTP_DIR, VETH_PREFIX,
    ZTP_DIR, ZTP_ISO, ZTP_JSON,
};
use shared::util;
use topology::{self, BridgeDetailed};

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
        .collect::<Result<Vec<topology::BridgeExpanded>>>()?;

    let mut bridges_detailed: Vec<topology::BridgeDetailed> = vec![];
    for (idx, bridge) in bridges.iter().enumerate() {
        let bridge_index = idx as u16;
        let manifest_name = bridge.name.clone();
        let bridge_name = format!("{}s{}-{}", BRIDGE_PREFIX, bridge_index, lab_id);
        let libvirt_name = format!("sherpa-bridge{}-{}-{}", bridge_index, bridge.name, lab_id);

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
        bridges_detailed.push(BridgeDetailed {
            manifest_name,
            bridge_name,
            libvirt_name,
            index: bridge_index,
            links: bridge_links,
        })
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

pub async fn up(
    sherpa: &data::Sherpa,
    qemu: &libvirt::Qemu,
    _lab_name: &str,
    lab_id: &str,
    manifest: &topology::Manifest,
) -> Result<()> {
    // Setup
    util::term_msg_surround(&format!("Building environment - {lab_id}"));

    let sherpa_user = util::sherpa_user()?;
    let lab_dir = format!("{SHERPA_BASE_DIR}/{SHERPA_LABS_DIR}/{lab_id}");
    let current_user = util::get_username()?;
    let management_network = format!("{}-{}", SHERPA_MANAGEMENT_NETWORK_NAME, lab_id);

    let docker_conn = container::docker_connection()?;
    let qemu_conn = Arc::new(qemu.connect()?);
    let db = db::connect(
        SHERPA_DB_SERVER,
        SHERPA_DB_PORT,
        SHERPA_DB_NAMESPACE,
        SHERPA_DB_NAME,
    )
    .await?;
    let db_user = db::get_user(&db, &current_user).await?;

    // Check if lab already exists
    if let Ok(lab) = db::get_lab(&db, lab_id).await {
        return Err(anyhow!(
            "Lab already exists. Please use a different lab ID or destory the existing lab first.\n Lab name: {}\n Lab id: {}",
            lab.name,
            lab_id,
        ));
    }

    println!("Loading config");
    let sherpa = sherpa.clone();

    let mut config = util::load_config(&sherpa.config_file_path)?;

    // Bulk fetch all node configs from database
    let node_configs = db::list_node_configs(&db).await?;

    util::term_msg_underline("Validating Manifest");

    // Device Validators
    validate::check_duplicate_device(&manifest.nodes)?;

    let nodes_expanded = process_manifest_nodes(&manifest.nodes);
    let links_detailed = process_manifest_links(&manifest.links, &nodes_expanded)?;
    let bridges_detailed = process_manifest_bridges(&manifest.bridges, &nodes_expanded, lab_id)?;
    let mut ztp_records = vec![];

    for node in &nodes_expanded {
        let node_config = get_node_config(&node.model, &node_configs)?;

        if !node_config.dedicated_management_interface {
            // Management interface is always at index 0 for non-dedicated management devices
            validate::check_mgmt_usage(&node.name, 0, &links_detailed)?;
        }

        validate::check_interface_bounds(
            &node.name,
            &node_config.model,
            node_config.data_interface_count,
            node_config.reserved_interface_count,
            node_config.dedicated_management_interface,
            &links_detailed,
        )?;
    }

    // Connection Validators
    if !links_detailed.is_empty() {
        validate::check_duplicate_interface_link(&links_detailed)?;
        validate::check_link_device(&manifest.nodes, &links_detailed)?;
    };

    println!("Manifest Ok");

    // Create lab record in database
    let lab_record = db::create_lab(&db, &manifest.name, lab_id, &db_user).await?;
    let lab_record_id = db::get_lab_id(&lab_record)?;

    let mut container_nodes: Vec<topology::NodeExpanded> = vec![];
    let mut unikernel_nodes: Vec<topology::NodeExpanded> = vec![];
    let mut vm_nodes: Vec<topology::NodeExpanded> = vec![];
    let mut clone_disks: Vec<data::CloneDisk> = vec![];
    let mut domains: Vec<template::DomainTemplate> = vec![];

    let mut lab_node_data = vec![];
    let mut node_setup_data = vec![];

    for node in nodes_expanded.iter() {
        // Look up the precise node config from HashMap using model+kind
        let node_config = get_node_config(&node.model, &node_configs)?;

        // Get a vector in node interfaces.
        // Process nodes to build a vector of a nodes links
        let mut node_interfaces_detailed: Vec<data::InterfaceData> = vec![];

        // It matters not, if the device has a dedicated MGMT interface.
        // The MGMT interface is always index 0. (The first interface)
        let _mgmt_interface_idx = 0;
        // The first data interface is either 1 or the first interface after the
        // number of reserved interfaces.
        let first_data_interface_idx = 1 + node_config.reserved_interface_count;

        // Calculate the maximum interface index to create based on data_interface_count
        // data_interface_count represents the number of data interfaces (not including mgmt/reserved)
        // Total interfaces = 1 (mgmt) + reserved_interface_count + data_interface_count
        let max_interface_idx = first_data_interface_idx + node_config.data_interface_count - 1;

        // Populate interface vector for only the configured number of interfaces
        for idx in 0..=max_interface_idx {
            let interface_name = util::interface_from_idx(&node.model, idx)?;

            let interface_idx = idx;
            let mut interface_state = data::InterfaceState::Enabled;
            let mut interface_data = data::NodeInterface::Disabled;

            if idx == 0 {
                // MGMT interface
                interface_data = data::NodeInterface::Management;
            } else if idx < first_data_interface_idx {
                // Reserved interface
                interface_data = data::NodeInterface::Reserved;
            } else {
                // Data interfaces

                // P2P Interface
                if let Some(data) =
                    find_interface_link(&node.name, &interface_name, &links_detailed)
                {
                    interface_data = data
                }

                // Bridge Interface
                if let Some(data) =
                    find_bridge_interface(&node.name, &interface_name, &bridges_detailed)
                {
                    interface_data = data
                }

                // All other interfaces are Disabled.
                interface_state = data::InterfaceState::Disabled;
            }

            // Create and add InterfaceData
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

        // Handle Containers, NanoVM's and regular VM's
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

        // All VM nodes have an isolated bridge created for unused interfaces.
        let node_isolated_network = if matches!(node_config.kind, data::NodeKind::VirtualMachine) {
            Some(node_isolated_network_data(&node.name, node.index, lab_id))
        } else {
            None
        };
        // If a VM has reserved interfaces create a reserved bridge
        let node_reserved_network = if matches!(node_config.kind, data::NodeKind::VirtualMachine)
            && node_config.reserved_interface_count > 0
        {
            Some(node_reserved_network_data(&node.name, node.index, lab_id))
        } else {
            None
        };

        // Create isolated network for this node (VMs and Unikernels only)
        // Containers will be handled separately in the future
        if let Some(network) = node_isolated_network.clone() {
            println!("Creating isolated network for node: {}", node.name);
            let node_isolated_network = libvirt::IsolatedNetwork {
                network_name: network.network_name,
                bridge_name: network.bridge_name,
            };
            node_isolated_network.create(&qemu_conn)?;
        };

        // Create reserved network for this node (VMs and Unikernels only)
        if let Some(network) = node_reserved_network.clone() {
            println!("Creating reserved network for node: {}", node.name);
            let node_reserved_network = libvirt::ReservedNetwork {
                network_name: network.network_name,
                bridge_name: network.bridge_name,
            };
            node_reserved_network.create(&qemu_conn)?;
        };

        // Store node setup data for later use in template::DomainTemplate creation
        node_setup_data.push(data::NodeSetupData {
            name: node.name.clone(),
            index: node.index,
            management_network: management_network.clone(),
            isolated_network: node_isolated_network,
            reserved_network: node_reserved_network,
            interfaces: node_interfaces_detailed,
        });
    }

    util::term_msg_underline("Lab Network");
    let lab_net = util::get_free_subnet(&config.management_prefix_ipv4.to_string())?;
    let gateway_ip = util::get_ipv4_addr(&lab_net, 1)?;
    let lab_router_ip = util::get_ipv4_addr(&lab_net, 2)?;
    let lab_info = data::LabInfo {
        id: lab_id.to_string(),
        user: util::get_username()?,
        name: manifest.name.clone(),
        ipv4_network: lab_net,
        ipv4_gateway: gateway_ip,
        ipv4_router: lab_router_ip,
    };

    println!("{}", lab_info);
    util::create_dir(&format!("{lab_dir}"))?;
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

    println!("Creating network: {SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}");
    // Libvirt networks
    let management_network = libvirt::NatNetwork {
        network_name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
        bridge_name: format!("{SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX}-{lab_id}"),
        ipv4_address: gateway_ip,
        ipv4_netmask: lab_net.netmask(),
    };
    management_network.create(&qemu_conn)?;

    // Docker Networks
    container::create_docker_bridge_network(
        &docker_conn,
        &format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
        Some(lab_net.to_string()),
        &format!("{SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX}-{lab_id}"),
    )
    .await?;

    let mut lab_link_data = vec![];

    // Point-to-Point links are created outside of libvirt. This allows
    // for better control of connections between VM's and Containers.
    // Each end of the connection has a bridge created, with a veth pair
    // connecting the bridges. This allows the targetting of the bridge
    // interface for packet captures.
    util::term_msg_underline("Creating Point-to-Point Links");
    for (idx, link) in links_detailed.iter().enumerate() {
        let node_a = lab_node_data
            .iter()
            .find(|n| n.name == link.node_a)
            .ok_or_else(|| anyhow!("Node not found: {}", link.node_a))?;

        let node_b = lab_node_data
            .iter()
            .find(|n| n.name == link.node_b)
            .ok_or_else(|| anyhow!("Node not found: {}", link.node_b))?;

        // Generate unique, names must fit within Linux interface name limits (15 chars)
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

        // Store link data for later use (still needed for infrastructure setup)
        let link_data = data::LabLinkData {
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
        };

        lab_link_data.push(link_data);

        println!(
            "Creating link #{} - {}::{} <-> {}::{}",
            idx, link.node_a, link.int_a, link.node_b, link.int_b
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
    }

    // Create Docker networks for container-connected bridges
    // This allows containers to attach to the pre-created Linux bridges
    util::term_msg_underline("Creating Docker Networks for Container Links");
    for link_data in &lab_link_data {
        // Look up node_a in lab_node_data to get its kind
        let node_a_data = lab_node_data
            .iter()
            .find(|n| n.record.id == link_data.node_a.id)
            .ok_or_else(|| anyhow!("Node A not found in lab_node_data"))?;

        // Look up node_b in lab_node_data to get its kind
        let node_b_data = lab_node_data
            .iter()
            .find(|n| n.record.id == link_data.node_b.id)
            .ok_or_else(|| anyhow!("Node B not found in lab_node_data"))?;

        // Create Docker network for node_a if it's a container
        if node_a_data.kind == data::NodeKind::Container {
            let docker_net_name =
                format!("{}-etha{}-{}", node_a_data.name, link_data.index, lab_id);
            container::create_docker_macvlan_network(
                &docker_conn,
                &link_data.bridge_a,
                &docker_net_name,
            )
            .await?;
        }

        // Create Docker network for node_b if it's a container
        if node_b_data.kind == data::NodeKind::Container {
            let docker_net_name =
                format!("{}-ethb{}-{}", node_b_data.name, link_data.index, lab_id);
            container::create_docker_macvlan_network(
                &docker_conn,
                &link_data.bridge_b,
                &docker_net_name,
            )
            .await?;
        }
    }

    // Create shared bridges for multi-host connections
    for bridge in bridges_detailed.iter() {
        util::term_msg_underline("Creating Shared Bridges");
        let mut bridge_nodes = vec![];

        println!(
            "Creating shared bridge #{} - {} ({} connections)",
            bridge.index,
            bridge.manifest_name,
            bridge.links.len()
        );

        network::create_bridge(&bridge.bridge_name, &bridge.libvirt_name).await?;

        for link in bridge.links.iter() {
            if let Some(node_data) = lab_node_data.iter().find(|n| n.name == link.node_name) {
                bridge_nodes.push(db::get_node_id(&node_data.record)?);
            }
        }
        // Create bridge record in database
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

    util::term_msg_underline("ZTP");
    if manifest.ztp_server.is_some() {
        config.ztp_server.enable = manifest.ztp_server.clone().unwrap().enable
    }
    if config.ztp_server.enable {
        println!("ZTP server is enabled in configuration")
    } else {
        println!("ZTP server is disabled in configuration")
    }

    // Containers
    for node in &mut container_nodes {
        let node_data = get_node_data(&node.name, &node_setup_data)?;
        let node_idx = node_data.index;

        let node_ip_idx = 10 + node_idx.to_owned() as u32;

        // generate the template
        println!("Creating container config: {}", node.name);
        let user = sherpa_user.clone();
        let dir = format!("{}/{}", lab_dir, node.name);

        node.ipv4_address = Some(util::get_ipv4_addr(&mgmt_net.v4.prefix, node_ip_idx)?);

        match node.model {
            data::NodeModel::AristaCeos => {
                let arista_template = template::AristaCeosZtpTemplate {
                    hostname: node.name.clone(),
                    user: user.clone(),
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
    // Unikernels

    // Virtual Machines
    for node in &vm_nodes {
        let node_data = get_node_data(&node.name, &node_setup_data)?;
        let node_idx = node_data.index;

        let node_ip_idx = 10 + node_idx.to_owned() as u32;

        let node_config = get_node_config(&node.model, &node_configs)?;

        let mut disks: Vec<data::NodeDisk> = vec![];
        let node_name = format!("{}-{}", node.name, lab_id);

        let hdd_bus = node_config.hdd_bus.clone();
        let cdrom_bus = node_config.cdrom_bus.clone();

        let mac_address = util::random_mac(KVM_OUI);
        ztp_records.push(data::ZtpRecord {
            node_name: node.name.clone().to_owned(),
            config_file: format!("{}.conf", &node.name),
            ipv4_address: util::get_ipv4_addr(&mgmt_net.v4.prefix, node_ip_idx)?,
            mac_address: mac_address.to_string(),
            ztp_method: node_config.ztp_method.clone(),
            ssh_port: 22,
        });

        let mut interfaces: Vec<data::Interface> = vec![];

        for interface in node_data.interfaces.iter() {
            match &interface.data {
                data::NodeInterface::Management => {
                    //
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
                    // TODO: Validate if this is the node id, not the interface id
                    let local_id = peer.this_node_index as u8;
                    let source_id = peer.peer_node_index as u8;

                    let interface_connection = data::InterfaceConnection {
                        local_id: peer.this_node_index,
                        local_port: util::id_to_port(local_id),
                        local_loopback: util::get_ip(local_id).to_string(),
                        source_id: peer.peer_node_index,
                        source_port: util::id_to_port(source_id),
                        source_loopback: util::get_ip(source_id).to_string(),
                    };
                    // TODO: This is from UDP P2P links add this functionality
                    // interfaces.push(data::Interface {
                    //     name: util::dasher(&l.int_a),
                    //     num: i,
                    //     mtu: node_model.interface_mtu,
                    //     mac_address: util::random_mac(KVM_OUI),
                    //     connection_type: data::ConnectionTypes::Peer,
                    //     interface_connection: Some(interface_connection),
                    // });

                    interfaces.push(data::Interface {
                        name: format!("{}a{}-{}", BRIDGE_PREFIX, peer.link_index, lab_id),
                        num: peer.this_interface_index,
                        mtu: node_config.interface_mtu,
                        mac_address: util::random_mac(KVM_OUI),
                        connection_type: data::ConnectionTypes::PeerBridge,
                        interface_connection: Some(interface_connection),
                    });
                }
                data::NodeInterface::Disabled => {
                    //
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
                    })
                }
            }
        }

        // Only Virtual machines have a boot disk to clone.
        let vm_boot_disk = match node_config.kind {
            data::NodeKind::VirtualMachine => {
                let src_boot_disk = format!(
                    "{}/{}/{}/virtioa.qcow2",
                    sherpa.images_dir, node_config.model, node_config.version
                );
                let dst_boot_disk = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name}-hdd.qcow2");

                clone_disks.push(data::CloneDisk {
                    src: src_boot_disk.clone(),
                    dst: dst_boot_disk.clone(),
                });

                Some(dst_boot_disk)
            }
            _ => None,
        };

        // CDROM ISO
        let (mut src_cdrom_iso, mut dst_cdrom_iso) = match &node_config.cdrom {
            Some(src_iso) => {
                let src = format!(
                    "{}/{}/{}/{}",
                    sherpa.images_dir, node_config.model, node_config.version, src_iso
                );
                let dst = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name}.iso");
                (Some(src), Some(dst))
            }
            None => (None, None),
        };

        // USB
        let (mut src_usb_disk, mut dst_usb_disk) = (None::<String>, None::<String>);

        // Config drive
        let (mut src_config_disk, mut dst_config_disk) = (None::<String>, None::<String>);

        // Ignition Config
        let (mut src_ignition_disk, mut dst_ignition_disk) = (None::<String>, None::<String>);

        if node_config.ztp_enable {
            // vm_nodes.push(node.clone());
            // TODO: Update this to use the assigned IP if
            // an IP is not user defined.
            let node_ipv4_address = ztp_records
                .iter()
                .find(|r| r.node_name == node.name)
                .map(|r| r.ipv4_address);
            match node_config.ztp_method {
                data::ZtpMethod::CloudInit => {
                    util::term_msg_underline("Creating Cloud-Init disks");
                    // generate the template
                    println!("Creating Cloud-Init config {}", node.name);
                    let dir = format!("{lab_dir}/{node_name}");
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

                            if node_ipv4_address.is_some() {
                                let ztp_interface = template::CloudInitNetwork::ztp_interface(
                                    // This should always be Some
                                    node_ipv4_address.unwrap(),
                                    mac_address,
                                    mgmt_net.v4.clone(),
                                );
                                let cloud_network_config = ztp_interface.to_string()?;
                                util::create_file(&network_config, cloud_network_config)?;
                            }

                            util::create_ztp_iso(&format!("{}/{}", dir, ZTP_ISO), dir)?
                        }

                        data::NodeModel::AlpineLinux => {
                            let meta_data = template::MetaDataConfig {
                                instance_id: format!("iid-{}", node.name.clone(),),
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

                            if node_ipv4_address.is_some() {
                                let ztp_interface = template::CloudInitNetwork::ztp_interface(
                                    // This should always be Some
                                    node_ipv4_address.unwrap(),
                                    mac_address,
                                    mgmt_net.v4.clone(),
                                );
                                let cloud_network_config = ztp_interface.to_string()?;
                                util::create_file(&network_config, cloud_network_config)?;
                            }

                            util::create_ztp_iso(&format!("{}/{}", dir, ZTP_ISO), dir)?
                        }
                        _ => {
                            anyhow::bail!(
                                "CDROM ZTP method not supported for {}",
                                node_config.model
                            );
                        }
                    }
                    src_cdrom_iso = Some(format!("{lab_dir}/{node_name}/{ZTP_ISO}"));
                    dst_cdrom_iso = Some(format!("{SHERPA_STORAGE_POOL_PATH}/{node_name}.iso"));
                }
                data::ZtpMethod::Cdrom => {
                    util::term_msg_underline("Creating ZTP disks");
                    // generate the template
                    println!("Creating ZTP config {}", node.name);
                    let mut user = sherpa_user.clone();
                    let dir = format!("{lab_dir}/{node_name}");

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
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let c = CISCO_IOSXE_ZTP_CONFIG.replace("-", "_");
                            let ztp_config = format!("{dir}/{c}");
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;
                            util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?
                        }
                        data::NodeModel::CiscoAsav => {
                            let key_hash =
                                util::pub_ssh_key_to_sha256_hash(&user.ssh_public_key.key)?;
                            user.ssh_public_key.key = key_hash;
                            let t = template::CiscoAsavZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                dns: dns.clone(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{CISCO_ASAV_ZTP_CONFIG}");
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;
                            util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?
                        }
                        data::NodeModel::CiscoNexus9300v => {
                            let t = template::CiscoNxosZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                dns: dns.clone(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{CISCO_NXOS_ZTP_CONFIG}");
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;
                            util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?
                        }
                        data::NodeModel::CiscoIosxrv9000 => {
                            let t = template::CiscoIosxrZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                dns: dns.clone(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{CISCO_IOSXR_ZTP_CONFIG}");
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;
                            util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?
                        }
                        data::NodeModel::CiscoFtdv => {
                            let t = template::CiscoFtdvZtpTemplate {
                                eula: "accept".to_string(),
                                hostname: node.name.clone(),
                                admin_password: SHERPA_PASSWORD.to_string(),
                                dns1: Some(mgmt_net.v4.boot_server),
                                ipv4_mode: Some(template::CiscoFxosIpMode::Manual),
                                ipv4_addr: node_ipv4_address,
                                ipv4_gw: Some(mgmt_net.v4.first),
                                ipv4_mask: Some(mgmt_net.v4.subnet_mask),
                                manage_locally: true,
                                ..Default::default()
                            };
                            let rendered_template = serde_json::to_string(&t)?;
                            let ztp_config = format!("{dir}/{CISCO_FTDV_ZTP_CONFIG}");
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;
                            util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?
                        }
                        data::NodeModel::JuniperVsrxv3
                        | data::NodeModel::JuniperVrouter
                        | data::NodeModel::JuniperVswitch => {
                            let t = template::JunipervJunosZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                mgmt_interface: node_config.management_interface.to_string(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{JUNIPER_ZTP_CONFIG}");
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;
                            util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?
                        }
                        _ => {
                            anyhow::bail!(
                                "CDROM ZTP method not supported for {}",
                                node_config.model
                            );
                        }
                    };
                    src_cdrom_iso = Some(format!("{lab_dir}/{node_name}/{ZTP_ISO}"));
                    dst_cdrom_iso = Some(format!("{SHERPA_STORAGE_POOL_PATH}/{node_name}-cfg.iso"));
                }
                data::ZtpMethod::Tftp => {
                    // generate the template
                    println!("Creating ZTP config {}", node.name);
                    let user = sherpa_user.clone();
                    let dir = format!("{lab_dir}/{ZTP_DIR}/{TFTP_DIR}");

                    match node.model {
                        data::NodeModel::AristaVeos => {
                            let arista_template = template::AristaVeosZtpTemplate {
                                hostname: node.name.clone(),
                                user: user.clone(),
                                dns: dns.clone(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = arista_template.render()?;
                            let ztp_config = format!("{dir}/{}.conf", node.name);
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;
                        }
                        data::NodeModel::ArubaAoscx => {
                            let aruba_template = template::ArubaAoscxTemplate {
                                hostname: node.name.clone(),
                                user: user.clone(),
                                dns: dns.clone(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let aruba_rendered_template = aruba_template.render()?;
                            let ztp_config = format!("{dir}/{}.conf", node.name);
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, aruba_rendered_template)?;
                        }
                        data::NodeModel::JuniperVevolved => {
                            let juniper_template = template::JunipervJunosZtpTemplate {
                                hostname: node.name.clone(),
                                user: sherpa_user.clone(),
                                mgmt_interface: node_config.management_interface.to_string(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let juniper_rendered_template = juniper_template.render()?;
                            let ztp_config = format!("{dir}/{}.conf", node.name);
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, juniper_rendered_template)?;
                        }
                        _ => {
                            anyhow::bail!(
                                "Tftp ZTP method not supported for {}",
                                node_config.model
                            );
                        }
                    }
                }
                data::ZtpMethod::Http => {
                    // generate the template
                    println!("Creating ZTP config {}", node.name);
                    let _user = sherpa_user.clone();
                    let dir = format!("{lab_dir}/{ZTP_DIR}/{NODE_CONFIGS_DIR}");

                    match node.model {
                        data::NodeModel::SonicLinux => {
                            let sonic_ztp_file_map = template::SonicLinuxZtp::file_map(
                                &node.name,
                                &mgmt_net.v4.boot_server,
                            );

                            let ztp_init = format!("{dir}/{}.conf", &node.name);
                            let sonic_ztp = template::SonicLinuxZtp {
                                hostname: node.name.clone(),
                                mgmt_ipv4: mgmt_net.v4.clone(),
                                mgmt_ipv4_address: node_ipv4_address,
                            };
                            let ztp_config = format!("{dir}/{}_config_db.json", &node.name);
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_init, sonic_ztp_file_map)?;
                            util::create_file(&ztp_config, sonic_ztp.config())?;
                        }
                        _ => {
                            anyhow::bail!(
                                "HTTP ZTP method not supported for {}",
                                node_config.model
                            );
                        }
                    }
                }
                data::ZtpMethod::Disk => {
                    println!("Creating ZTP config {}", node.name);
                    let mut user = sherpa_user.clone();

                    let dir = format!("{lab_dir}/{node_name}");
                    match node.model {
                        data::NodeModel::CiscoIosv => {
                            let key_hash = util::pub_ssh_key_to_md5_hash(&user.ssh_public_key.key)?;
                            user.ssh_public_key.key = key_hash;
                            let t = template::CiscoIosvZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                mgmt_interface: node_config.management_interface.to_string(),
                                dns: dns.clone(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let c = CISCO_IOSV_ZTP_CONFIG;
                            let ztp_config = format!("{dir}/{c}");
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;
                            // clone disk
                            let src_disk = format!(
                                "{}/{}/{}",
                                &sherpa.images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_IOSV
                            );
                            let dst_disk = format!("{dir}/{node_name}-cfg.img");

                            // Create a copy of the disk base image
                            util::copy_file(&src_disk, &dst_disk)?;
                            // copy file to disk disk
                            util::copy_to_dos_image(&ztp_config, &dst_disk, "/")?;

                            src_config_disk = Some(dst_disk.to_owned());
                            dst_config_disk =
                                Some(format!("{SHERPA_STORAGE_POOL_PATH}/{node_name}-cfg.img"));
                        }
                        data::NodeModel::CiscoIosvl2 => {
                            let key_hash = util::pub_ssh_key_to_md5_hash(&user.ssh_public_key.key)?;
                            user.ssh_public_key.key = key_hash;
                            let t = template::CiscoIosvl2ZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                mgmt_interface: node_config.management_interface.to_string(),
                                dns: dns.clone(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let c = CISCO_IOSV_ZTP_CONFIG;
                            let ztp_config = format!("{dir}/{c}");
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;
                            // clone disk
                            let src_disk = format!(
                                "{}/{}/{}",
                                &sherpa.images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_IOSV
                            );
                            let dst_disk = format!("{dir}/{node_name}-cfg.img");

                            // Create a copy of the hdd base image
                            util::copy_file(&src_disk, &dst_disk)?;
                            // copy file to hdd disk
                            util::copy_to_dos_image(&ztp_config, &dst_disk, "/")?;

                            src_config_disk = Some(dst_disk.to_owned());
                            dst_config_disk =
                                Some(format!("{SHERPA_STORAGE_POOL_PATH}/{node_name}-cfg.img"));
                        }
                        data::NodeModel::CiscoIse => {
                            let t = template::CiscoIseZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                dns: dns.clone(),
                                mgmt_ipv4_address: node_ipv4_address.ok_or_else(|| anyhow!("Cisco ISE node model requires an IPv4 management address. Node: {}", node.name))?,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{CISCO_ISE_ZTP_CONFIG}");
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;

                            // clone disk
                            let src_disk = format!(
                                "{}/{}/{}",
                                &sherpa.images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_ISE
                            );
                            let dst_disk = format!("{dir}/{node_name}-cfg.img");

                            // Create a copy of the hdd base image
                            util::copy_file(&src_disk, &dst_disk)?;
                            // copy file to hdd disk
                            util::copy_to_ext4_image(vec![&ztp_config], &dst_disk, "/")?;

                            src_config_disk = Some(dst_disk.to_owned());
                            dst_config_disk =
                                Some(format!("{SHERPA_STORAGE_POOL_PATH}/{node_name}-cfg.img"));
                        }
                        _ => {
                            anyhow::bail!(
                                "Disk ZTP method not supported for {}",
                                node_config.model
                            );
                        }
                    }
                }
                data::ZtpMethod::Usb => {
                    // generate the template
                    println!("Creating ZTP config {}", node.name);
                    let user = sherpa_user.clone();
                    let dir = format!("{lab_dir}/{node_name}");

                    match node_config.model {
                        data::NodeModel::CumulusLinux => {
                            let t = template::CumulusLinuxZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                dns: dns.clone(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{CUMULUS_ZTP}");
                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;
                            // clone USB disk
                            let src_usb = format!(
                                "{}/{}/{}",
                                &sherpa.images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_FAT32
                            );

                            let dst_usb = format!("{dir}/cfg.img");

                            // Create a copy of the usb base image
                            util::copy_file(&src_usb, &dst_usb)?;
                            // copy file to USB disk
                            util::copy_to_dos_image(&ztp_config, &dst_usb, "/")?;

                            src_usb_disk = Some(dst_usb.to_owned());
                            dst_usb_disk =
                                Some(format!("{SHERPA_STORAGE_POOL_PATH}/{node_name}-cfg.img"));
                        }
                        data::NodeModel::JuniperVevolved => {
                            let t = template::JunipervJunosZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                mgmt_interface: node_config.management_interface.to_string(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{JUNIPER_ZTP_CONFIG}");
                            let ztp_config_tgz = format!("{dir}/{JUNIPER_ZTP_CONFIG_TGZ}");

                            util::create_dir(&dir)?;
                            util::create_file(&ztp_config, rendered_template)?;
                            // clone USB disk
                            let src_usb = format!(
                                "{}/{}/{}",
                                &sherpa.images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_JUNOS
                            );
                            let dst_usb = format!("{dir}/cfg.img");

                            // Create a copy of the usb base image
                            util::copy_file(&src_usb, &dst_usb)?;

                            // Create tar.gz config file
                            util::create_config_archive(&ztp_config, &ztp_config_tgz)?;

                            // copy file to USB disk
                            util::copy_to_dos_image(&ztp_config_tgz, &dst_usb, "/")?;
                            // util::copy_to_dos_image(&ztp_config, &dst_usb, "/")?;

                            src_usb_disk = Some(dst_usb.to_owned());
                            dst_usb_disk =
                                Some(format!("{SHERPA_STORAGE_POOL_PATH}/{node_name}-cfg.img"));
                        }
                        _ => {
                            anyhow::bail!("USB ZTP method not supported for {}", node_config.model);
                        }
                    }
                }
                data::ZtpMethod::Ignition => {
                    util::term_msg_underline("Creating ZTP disks");
                    // generate the template
                    println!("Creating ZTP config {}", node.name);
                    let user = sherpa_user.clone();
                    let dir = format!("{lab_dir}/{node_name}");
                    let dev_name = node.name.clone();
                    // Add the ignition config

                    let mut authorized_keys = vec![format!(
                        "{} {} {}",
                        user.ssh_public_key.algorithm,
                        user.ssh_public_key.key,
                        user.ssh_public_key.comment.unwrap_or("".to_owned())
                    )];

                    let manifest_authorized_keys: Vec<String> =
                        node.ssh_authorized_keys.clone().unwrap_or(vec![]);

                    let manifest_authorized_key_files: Vec<String> = node
                        .ssh_authorized_key_files
                        .iter() // Iterator over Option<Vec<File>>
                        .flatten() // Flattens Option<Vec<File>> to individual &File items
                        .map(|file| -> Result<String> {
                            // file is now &File
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
                        contents: template::IgnitionFileContents::new(
                            &format!("data:,{dev_name}",),
                        ),
                        ..Default::default()
                    };
                    // files
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
                        .iter() // Iterator over Option<Vec<File>>
                        .flatten() // Flattens Option<Vec<File>> to individual &File items
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
                        .iter() // Iterator over Option<Vec<File>>
                        .flatten() // Flattens Option<Vec<File>> to individual &File items
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

                    match node.model {
                        data::NodeModel::FlatcarLinux => {
                            let mut units = vec![];
                            units.push(template::IgnitionUnit::mount_container_disk());
                            units.extend(manifest_systemd_units);

                            let container_disk = template::IgnitionFileSystem::default();

                            let mut files = vec![sudo_config_file, hostname_file, disable_update];
                            files.extend(manifest_text_files);

                            if node_ipv4_address.is_some() {
                                files.push(template::IgnitionFile::ztp_interface(
                                    // This should always be Some
                                    node_ipv4_address.unwrap(),
                                    mgmt_net.v4.clone(),
                                )?);
                            }

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
                                format!("{SHERPA_STORAGE_POOL_PATH}/{node_name}-cfg.ign");

                            util::create_dir(&dir)?;
                            util::create_file(&src_ztp_file, flatcar_config)?;

                            // Copy a blank disk to to .tmp directory
                            let src_data_disk = format!(
                                "{}/{}/{}",
                                &sherpa.images_dir,
                                SHERPA_BLANK_DISK_DIR,
                                SHERPA_BLANK_DISK_EXT4_500MB
                            );
                            let dst_disk = format!("{dir}/{node_name}-{CONTAINER_DISK_NAME}");

                            util::copy_file(&src_data_disk, &dst_disk)?;

                            let disk_files: Vec<&str> = manifest_binary_disk_files
                                .iter()
                                .map(|x| x.source.as_str())
                                .collect();

                            // Copy to container image into the container disk
                            if !disk_files.is_empty() {
                                util::copy_to_ext4_image(disk_files, &dst_disk, "/")?;
                            }

                            src_config_disk = Some(dst_disk.to_owned());
                            dst_config_disk = Some(format!(
                                "{SHERPA_STORAGE_POOL_PATH}/{node_name}-{CONTAINER_DISK_NAME}"
                            ));

                            src_ignition_disk = Some(src_ztp_file.to_owned());
                            dst_ignition_disk = Some(dst_ztp_file.to_owned());
                        }
                        _ => {
                            anyhow::bail!(
                                "Ignition ZTP method not supported for {}",
                                node_config.model
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        // ISO
        if let (Some(src_cdrom_iso), Some(dst_cdrom_iso)) = (src_cdrom_iso, dst_cdrom_iso) {
            clone_disks.push(data::CloneDisk {
                // These should always have a value.
                src: src_cdrom_iso,
                dst: dst_cdrom_iso.clone(),
            });
            disks.push(data::NodeDisk {
                disk_device: data::DiskDevices::Cdrom,
                driver_name: data::DiskDrivers::Qemu,
                driver_format: data::DiskFormats::Raw,
                src_file: dst_cdrom_iso.clone(),
                target_dev: data::DiskTargets::target(&cdrom_bus, disks.len() as u8)?,
                target_bus: cdrom_bus.clone(),
            });
        }

        // Hdd
        if let Some(vm_boot_disk) = vm_boot_disk {
            disks.push(data::NodeDisk {
                disk_device: data::DiskDevices::File,
                driver_name: data::DiskDrivers::Qemu,
                driver_format: data::DiskFormats::Qcow2,
                src_file: vm_boot_disk.clone(),
                target_dev: data::DiskTargets::target(&hdd_bus, disks.len() as u8)?,
                target_bus: hdd_bus.clone(),
            });
        }

        // Data Disk
        if let (Some(src_config_disk), Some(dst_config_disk)) = (src_config_disk, dst_config_disk) {
            clone_disks.push(data::CloneDisk {
                src: src_config_disk,
                dst: dst_config_disk.clone(),
            });
            disks.push(data::NodeDisk {
                disk_device: data::DiskDevices::File,
                driver_name: data::DiskDrivers::Qemu,
                driver_format: data::DiskFormats::Raw,
                src_file: dst_config_disk.clone(),
                target_dev: data::DiskTargets::target(&hdd_bus, disks.len() as u8)?,
                target_bus: hdd_bus.clone(),
            });
        }

        // USB
        if let (Some(src_usb_disk), Some(dst_usb_disk)) = (src_usb_disk, dst_usb_disk) {
            clone_disks.push(data::CloneDisk {
                src: src_usb_disk,
                dst: dst_usb_disk.clone(),
            });
            disks.push(data::NodeDisk {
                disk_device: data::DiskDevices::File,
                driver_name: data::DiskDrivers::Qemu,
                driver_format: data::DiskFormats::Raw,
                src_file: dst_usb_disk.clone(),
                target_dev: data::DiskTargets::target(&data::DiskBuses::Usb, disks.len() as u8)?,
                target_bus: data::DiskBuses::Usb,
            });
        }

        // Ignition
        if let (Some(src_ignition_disk), Some(dst_ignition_disk)) =
            (src_ignition_disk, dst_ignition_disk.clone())
        {
            clone_disks.push(data::CloneDisk {
                src: src_ignition_disk,
                dst: dst_ignition_disk.clone(),
            });
            disks.push(data::NodeDisk {
                disk_device: data::DiskDevices::File,
                driver_name: data::DiskDrivers::Qemu,
                driver_format: data::DiskFormats::Raw,
                src_file: dst_ignition_disk.clone(),
                target_dev: data::DiskTargets::target(&data::DiskBuses::Sata, disks.len() as u8)?,
                target_bus: data::DiskBuses::Sata,
            });
        }

        let qemu_commands = match node_config.model {
            data::NodeModel::JuniperVrouter => data::QemuCommand::juniper_vrouter(),
            data::NodeModel::JuniperVswitch => data::QemuCommand::juniper_vswitch(),
            data::NodeModel::JuniperVevolved => data::QemuCommand::juniper_vevolved(),
            data::NodeModel::FlatcarLinux => {
                if let Some(dst_ignition_disk) = dst_ignition_disk {
                    data::QemuCommand::ignition_config(&dst_ignition_disk)
                } else {
                    vec![]
                }
            }
            _ => {
                vec![]
            }
        };

        let node_id = node_data.index;

        if node_config.kind == data::NodeKind::VirtualMachine {
            // Get the network names for this node from NodeSetupData
            let node_data = get_node_data(&node.name, &node_setup_data)?;

            let management_network = node_data.management_network.clone();

            let isolated_network = node_data
                .isolated_network
                .clone()
                .ok_or_else(|| anyhow!("Isolated network not found for VM node: {}", node.name))?;

            let reserved_network =
                if let Some(reserved_network) = node_data.reserved_network.clone() {
                    reserved_network.network_name
                } else {
                    "".to_string()
                };

            let domain = template::DomainTemplate {
                qemu_bin: config.qemu_bin.clone(),
                name: node_name,
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
                loopback_ipv4: util::get_ip(node_id as u8).to_string(),
                telnet_port: TELNET_PORT,
                qemu_commands,
                lab_id: lab_id.to_string(),
                management_network,
                isolated_network: isolated_network.network_name,
                reserved_network,
            };
            domains.push(domain);
        }
    }

    create_ztp_files(&mgmt_net, &sherpa_user, &lab_id, &ztp_records)?;
    create_boot_containers(&docker_conn, &mgmt_net, lab_id).await?;

    // Clone disks in parallel
    util::term_msg_underline("Cloning Disks");
    let disk_handles: Vec<_> = clone_disks
        .into_iter()
        .map(|disk| {
            let qemu_conn = Arc::clone(&qemu_conn);
            thread::spawn(move || -> Result<()> {
                println!("Cloning disk \n  from: {} \n    to: {}", disk.src, disk.dst);
                libvirt::clone_disk(&qemu_conn, &disk.src, &disk.dst).with_context(|| {
                    format!("Failed to clone disk from: {} to: {}", disk.src, disk.dst)
                })?;
                println!("Cloned disk \n  from: {} \n    to: {}", disk.src, disk.dst);
                Ok(())
            })
        })
        .collect();

    // Wait for all threads to complete
    for handle in disk_handles {
        handle
            .join()
            .map_err(|e| anyhow!("Error cloning disk: {:?}", e))??;
    }

    // Build domains in parallel
    util::term_msg_underline("Creating Node Configs");

    let vm_handles: Vec<_> = domains
        .into_iter()
        .map(|domain| {
            let qemu_conn = Arc::clone(&qemu_conn);
            thread::spawn(move || -> Result<()> {
                let rendered_xml = domain
                    .render()
                    .with_context(|| format!("Failed to render XML for VM: {}", domain.name))?;

                println!("Creating VM: {}", domain.name);
                libvirt::create_vm(&qemu_conn, &rendered_xml)
                    .with_context(|| format!("Failed to create VM: {}", domain.name))?;
                println!("Created VM: {}", domain.name);
                Ok(())
            })
        })
        .collect();

    // Wait for all threads to complete
    for handle in vm_handles {
        handle
            .join()
            .map_err(|e| anyhow!("Error creating VM: {:?}", e))??;
    }

    if !ztp_records.is_empty() {
        // Check manifest's config_management setting, defaulting to false if not present
        let _pyats_enabled = manifest
            .config_management
            .as_ref()
            .map(|c| c.pyats)
            .unwrap_or(false);

        // if _pyats_enabled {
        //     util::term_msg_underline("Creating PyATS Testbed File");
        //     let pyats_inventory = template::PyatsInventory::from_manifest(
        //         manifest,
        //         &node_configs,
        //         &ztp_records,
        //         config.ztp_server.username.clone(),
        //         config.ztp_server.password.clone(),
        //     )?;
        //     let pyats_yaml = pyats_inventory.to_yaml()?;
        //     util::create_file(&format!("{lab_dir}/testbed.yaml"), pyats_yaml)?;
        // }

        util::term_msg_underline("Creating SSH Config File");
        
        // Load server config to get server_ipv4
        let config_contents = util::load_file(&sherpa.config_file_path)
            .context("Failed to load sherpa.toml config")?;
        let config: data::Config = toml::from_str(&config_contents)
            .context("Failed to parse sherpa.toml config")?;
        
        let ssh_config_template = template::SshConfigTemplate {
            ztp_records: ztp_records.clone(),
            proxy_user: current_user.clone(),
            server_ipv4: config.server_ipv4.to_string(),
        };
        let rendered_template = ssh_config_template.render()?;
        util::create_file(
            &format!("{lab_dir}/{SHERPA_SSH_CONFIG_FILE}"),
            rendered_template,
        )?;
    }

    // Build container network attachment map for p2p links
    // Maps container name -> list of Docker networks to attach
    util::term_msg_underline("Building Container Link Network Map");
    let mut container_link_networks: HashMap<String, Vec<data::ContainerNetworkAttachment>> =
        HashMap::new();

    for link_data in &lab_link_data {
        // Look up node kinds
        let node_a_data = lab_node_data
            .iter()
            .find(|n| n.record.id == link_data.node_a.id)
            .ok_or_else(|| anyhow!("Node A not found in lab_node_data"))?;

        let node_b_data = lab_node_data
            .iter()
            .find(|n| n.record.id == link_data.node_b.id)
            .ok_or_else(|| anyhow!("Node B not found in lab_node_data"))?;

        // Add network attachment for node_a if it's a container
        if node_a_data.kind == data::NodeKind::Container {
            let docker_net_name =
                format!("{}-etha{}-{}", node_a_data.name, link_data.index, lab_id);

            container_link_networks
                .entry(node_a_data.name.clone())
                .or_insert_with(Vec::new)
                .push(data::ContainerNetworkAttachment {
                    name: docker_net_name,
                    ipv4_address: None, // No IP - pure L2 connection
                });
        }

        // Add network attachment for node_b if it's a container
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

    // Sort the network attachments for each container to ensure deterministic interface ordering
    // for networks in container_link_networks.values_mut() {
    //     networks.sort_by(|a, b| a.name.cmp(&b.name));
    // }

    // Check if VMs are ready
    util::term_msg_underline("Checking Node Readiness");
    let start_time = Instant::now();
    let timeout = Duration::from_secs(READINESS_TIMEOUT); // 10 minutes
    let mut connected_nodes = std::collections::HashSet::new();
    let mut node_ip_map = vec![];

    let all_lab_nodes = vec![
        container_nodes.clone(),
        unikernel_nodes.clone(),
        vm_nodes.clone(),
    ]
    .concat();
    let total_lab_nodes = all_lab_nodes.len();

    println!(
        "Waiting for Nodes: {}",
        &all_lab_nodes
            .iter()
            .map(|x| x.name.as_str())
            .collect::<Vec<&str>>()
            .join(" ")
    );

    while start_time.elapsed() < timeout && connected_nodes.len() < total_lab_nodes {
        // Containers
        for container in &container_nodes {
            if connected_nodes.contains(&container.name) {
                continue;
            }
            let mgmt_ipv4 = container.ipv4_address.map(|i| i.to_string());
            let container_name = format!("{}-{}", container.name, lab_id);
            // TODO: FIX THESE UNWRAPS
            let container_image = format!(
                "{}:{}",
                container.image.as_ref().unwrap(),
                container.version.as_ref().unwrap()
            );
            let privileged = container.privileged.clone().unwrap_or_else(|| false);
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

            let management_network = data::ContainerNetworkAttachment {
                name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
                ipv4_address: mgmt_ipv4,
            };

            // Build network attachments for this container
            let mut additional_networks = vec![];

            // Add p2p link networks if any exist for this container
            if let Some(link_networks) = container_link_networks.get(&container.name) {
                additional_networks.extend_from_slice(link_networks);
            }

            container::run_container(
                //
                &docker_conn,
                &container_name,
                &container_image,
                env_vars,
                volumes,
                vec![],
                management_network,
                additional_networks,
                commands,
                privileged,
            )
            .await?;
            connected_nodes.insert(container.name.clone());
        }

        // Unikernels

        // Virtual Machines
        for vm in &vm_nodes {
            if connected_nodes.contains(&vm.name) {
                continue;
            }

            if let Some(vm_data) = ztp_records.iter().find(|x| x.node_name == vm.name) {
                match validate::tcp_connect(&vm_data.ipv4_address.to_string(), SSH_PORT)? {
                    true => {
                        println!("{} - Ready", &vm.name);
                        connected_nodes.insert(vm.name.clone());
                        node_ip_map.push(data::NodeConnection {
                            name: vm.name.clone(),
                            ip_address: vm_data.ipv4_address.to_string(),
                            ssh_port: SSH_PORT,
                        });
                    }
                    false => {
                        println!("{} - Waiting for SSH", vm.name);
                    }
                }
                // let leases = get_dhcp_leases(&config).await?;
                // if let Some(lease) = leases
                //     .iter()
                //     .find(|d| clean_mac(&d.mac_address) == clean_mac(&vm_data.mac_address))
                // {
                //     match tcp_connect(&lease.ipv4_address, ssh_port)? {
                //         true => {
                //             println!("{} - Ready", &node.name);
                //             connected_nodes.insert(node.name.clone());
                //             node_ip_map.push(NodeConnection {
                //                 name: node.name.clone(),
                //                 ip_address: lease.ipv4_address.clone(),
                //                 ssh_port,
                //             });
                //         }
                //         false => {
                //             println!("{} - Waiting for SSH", node.name);
                //         }
                //     }
                // } else {
                //     println!("{} - Still booting.", node.name);
                // }
            }
        }

        if connected_nodes.len() < total_lab_nodes {
            sleep(Duration::from_secs(READINESS_SLEEP));
        }
    }

    if connected_nodes.len() == total_lab_nodes {
        println!("All nodes are ready!");
    } else {
        println!("Timeout reached. Not all nodes are ready.");
        for node in &vm_nodes {
            if !connected_nodes.contains(&node.name) {
                println!("Node is not ready: {}", node.name);
            }
        }
    }

    Ok(())
}
