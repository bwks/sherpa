use super::boot_containers::{create_boot_containers, create_ztp_files};
use anyhow::{Context, Result, anyhow};
use askama::Template;

use container::{create_docker_bridge_network, docker_connection, run_container};
use data::{
    BridgeKind, CloneDisk, ConnectionTypes, ContainerNetworkAttachment, DiskBuses, DiskDevices,
    DiskDrivers, DiskFormats, DiskTargets, Interface, InterfaceConnection, LabInfo, LabLinkData,
    LabNodeData, NetworkV4, NodeConnection, NodeDisk, NodeKind, NodeModel, OsVariant, QemuCommand,
    Sherpa, SherpaNetwork, ZtpMethod, ZtpRecord,
};
use db::{connect, create_lab, create_link, create_node, get_node_config_by_model_kind, get_user};
use konst::{
    ARISTA_CEOS_ZTP_VOLUME_MOUNT, BRIDGE_PREFIX, CISCO_ASAV_ZTP_CONFIG, CISCO_FTDV_ZTP_CONFIG,
    CISCO_IOSV_ZTP_CONFIG, CISCO_IOSXE_ZTP_CONFIG, CISCO_IOSXR_ZTP_CONFIG, CISCO_ISE_ZTP_CONFIG,
    CISCO_NXOS_ZTP_CONFIG, CLOUD_INIT_META_DATA, CLOUD_INIT_NETWORK_CONFIG, CLOUD_INIT_USER_DATA,
    CONTAINER_ARISTA_CEOS_COMMANDS, CONTAINER_ARISTA_CEOS_ENV_VARS, CONTAINER_ARISTA_CEOS_REPO,
    CONTAINER_DISK_NAME, CONTAINER_NOKIA_SRLINUX_COMMANDS, CONTAINER_NOKIA_SRLINUX_ENV_VARS,
    CONTAINER_NOKIA_SRLINUX_REPO, CONTAINER_SURREAL_DB_COMMANDS, CONTAINER_SURREAL_DB_REPO,
    CUMULUS_ZTP, JUNIPER_ZTP_CONFIG, JUNIPER_ZTP_CONFIG_TGZ, KVM_OUI, LAB_FILE_NAME,
    NODE_CONFIGS_DIR, READINESS_SLEEP, READINESS_TIMEOUT, SHERPA_BASE_DIR, SHERPA_BLANK_DISK_DIR,
    SHERPA_BLANK_DISK_EXT4_500MB, SHERPA_BLANK_DISK_FAT32, SHERPA_BLANK_DISK_IOSV,
    SHERPA_BLANK_DISK_ISE, SHERPA_BLANK_DISK_JUNOS, SHERPA_DOMAIN_NAME,
    SHERPA_ISOLATED_NETWORK_BRIDGE_PREFIX, SHERPA_ISOLATED_NETWORK_NAME, SHERPA_LABS_DIR,
    SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX, SHERPA_MANAGEMENT_NETWORK_NAME, SHERPA_PASSWORD,
    SHERPA_PASSWORD_HASH, SHERPA_SSH_CONFIG_FILE, SHERPA_STORAGE_POOL_PATH, SHERPA_USERNAME,
    SSH_PORT, TELNET_PORT, TFTP_DIR, VETH_PREFIX, ZTP_DIR, ZTP_ISO, ZTP_JSON,
};
use libvirt::{IsolatedNetwork, NatNetwork, Qemu, clone_disk, create_vm};
use network::{create_bridge, create_veth_pair, enslave_to_bridge};
use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::thread::sleep;
use std::time::{Duration, Instant};
use template::{
    AristaCeosZtpTemplate, AristaVeosZtpTemplate, ArubaAoscxTemplate, CiscoAsavZtpTemplate,
    CiscoFtdvZtpTemplate, CiscoFxosIpMode, CiscoIosXeZtpTemplate, CiscoIosvZtpTemplate,
    CiscoIosvl2ZtpTemplate, CiscoIosxrZtpTemplate, CiscoIseZtpTemplate, CiscoNxosZtpTemplate,
    CloudInitConfig, CloudInitNetwork, CloudInitUser, Contents as IgnitionFileContents,
    CumulusLinuxZtpTemplate, DomainTemplate, File as IgnitionFile,
    FileParams as IgnitionFileParams, FileSystem as IgnitionFileSystem, IgnitionConfig,
    JunipervJunosZtpTemplate, MetaDataConfig, PyatsInventory, SonicLinuxZtp, SshConfigTemplate,
    Unit as IgnitionUnit, User as IgnitionUser,
};
use topology::{LinkDetailed, LinkExpanded, Manifest, Node, VolumeMount};
use util::{
    base64_encode, base64_encode_file, copy_file, copy_to_dos_image, copy_to_ext4_image,
    create_config_archive, create_dir, create_file, create_ztp_iso, dasher, default_dns,
    get_free_subnet, get_ip, get_ipv4_addr, get_ssh_public_key, get_username, id_to_port,
    interface_from_idx, interface_to_idx, load_config, load_file, pub_ssh_key_to_md5_hash,
    pub_ssh_key_to_sha256_hash, random_mac, sherpa_user, term_msg_surround, term_msg_underline,
};
use validate::{
    check_duplicate_device, check_duplicate_interface_link, check_interface_bounds,
    check_link_device, check_mgmt_usage, tcp_connect,
};

pub async fn up(
    sherpa: &Sherpa,
    qemu: &Qemu,
    _lab_name: &str,
    lab_id: &str,
    manifest: &Manifest,
) -> Result<()> {
    // Setup
    let docker_conn = docker_connection()?;
    let qemu_conn = Arc::new(qemu.connect()?);
    let sherpa_user = sherpa_user()?;
    let lab_dir = format!("{SHERPA_BASE_DIR}/{SHERPA_LABS_DIR}/{lab_id}");

    term_msg_surround(&format!("Building environment - {lab_id}"));

    println!("Loading config");
    let sherpa = sherpa.clone();

    let mut config = load_config(&sherpa.config_file_path)?;

    // TODO: RUN EXISTING LAB VALIDATORS

    term_msg_underline("Validating Manifest");
    let manifest_links = manifest.links.clone().unwrap_or_default();

    // links from manifest links
    let links = manifest_links
        .iter()
        .map(|x| x.expand())
        .collect::<Result<Vec<LinkExpanded>>>()?;

    let mut links_detailed = vec![];
    for (link_idx, link) in links.iter().enumerate() {
        let mut this_link = LinkDetailed::default();
        for device in manifest.nodes.iter() {
            let device_model = device.model.clone();
            if link.node_a == device.name {
                let int_idx = interface_to_idx(&device_model, &link.int_a)?;
                this_link.node_a = device.name.clone();
                this_link.node_a_model = device_model;
                this_link.int_a = link.int_a.clone();
                this_link.int_a_idx = int_idx;
                this_link.link_idx = link_idx as u32
            } else if link.node_b == device.name {
                let int_idx = interface_to_idx(&device_model, &link.int_b)?;
                this_link.node_b = device.name.clone();
                this_link.node_b_model = device_model;
                this_link.int_b = link.int_b.clone();
                this_link.int_b_idx = int_idx;
                this_link.link_idx = link_idx as u32
            }
        }
        links_detailed.push(this_link)
    }

    // Device Validators
    check_duplicate_device(&manifest.nodes)?;

    let mut ztp_records = vec![];

    for node in &manifest.nodes {
        let node_model = config
            .node_config
            .iter()
            .find(|d| d.model == node.model)
            .ok_or_else(|| anyhow::anyhow!("Device model not found: {}", node.model))?;

        if !node_model.dedicated_management_interface {
            check_mgmt_usage(
                &node.name,
                node_model.first_interface_index,
                &links_detailed,
            )?;
        }

        check_interface_bounds(
            &node.name,
            &node_model.model,
            node_model.first_interface_index,
            node_model.interface_count,
            &links_detailed,
        )?;
    }

    // Connection Validators
    if !links.is_empty() {
        check_duplicate_interface_link(&links_detailed)?;
        check_link_device(&manifest.nodes, &links)?;
    };

    println!("Manifest Ok");

    // Testing
    // Connect to DB
    let db = connect("localhost", 8000, "test", "test").await?;
    let db_user = get_user(&db, "bradmin").await?;
    let lab_record = create_lab(&db, &manifest.name, lab_id, &db_user).await?;

    // Create a mapping of node name to node id.
    // Nodes have an id based on their order in the list of nodes
    // from the manifest file.
    let node_id_map: HashMap<String, u16> = manifest
        .nodes
        .iter()
        .enumerate()
        .map(|(idx, device)| (device.name.clone(), idx as u16 + 1))
        .collect();

    let mut container_nodes: Vec<Node> = vec![];
    let mut unikernel_nodes: Vec<Node> = vec![];
    let mut vm_nodes: Vec<Node> = vec![];
    let mut clone_disks: Vec<CloneDisk> = vec![];
    let mut domains: Vec<DomainTemplate> = vec![];

    let mut lab_node_data = vec![];

    for node in &manifest.nodes {
        let node_idx = node_id_map
            .get(&node.name)
            .ok_or_else(|| anyhow::anyhow!("Node not found in node ID map: {}", node.name))?;

        let node_data = config
            .node_config
            .iter()
            .find(|d| d.model == node.model)
            .ok_or_else(|| anyhow::anyhow!("Node model not found: {}", node.model))?;

        // Look up the node config in the database
        let node_config = get_node_config_by_model_kind(&db, &node.model, &node_data.kind.to_string())
            .await?
            .ok_or_else(|| anyhow::anyhow!("Node config not found in database for model: {}", node.model))?;
        
        let lab_node = create_node(
            &db,
            &node.name,
            *node_idx,
            node_config.id.ok_or_else(|| anyhow::anyhow!("Config has no ID"))?,
            lab_record.id.clone().ok_or_else(|| anyhow::anyhow!("Lab has no ID"))?,
        ).await?;

        lab_node_data.push(LabNodeData {
            name: node.name.clone(),
            model: node_data.model.clone(),
            kind: node_data.kind.clone(),
            index: *node_idx,
            record: lab_node,
        });

        // Handle Containers, NanoVM's and regular VM's
        match node_data.kind {
            NodeKind::Container => {
                container_nodes.push(node.clone());
            }
            NodeKind::Unikernel => {
                unikernel_nodes.push(node.clone());
            }
            NodeKind::VirtualMachine => {
                vm_nodes.push(node.clone());
            }
        }
    }

    term_msg_underline("Lab Network");
    let lab_net = get_free_subnet(&config.management_prefix_ipv4.to_string())?;
    let gateway_ip = get_ipv4_addr(&lab_net, 1)?;
    let lab_router_ip = get_ipv4_addr(&lab_net, 2)?;
    let lab_info = LabInfo {
        id: lab_id.to_string(),
        user: get_username()?,
        name: manifest.name.clone(),
        ipv4_network: lab_net,
        ipv4_gateway: gateway_ip,
        ipv4_router: lab_router_ip,
    };

    println!("{}", lab_info);
    create_dir(&format!("{lab_dir}"))?;
    create_file(&format!("{lab_dir}/{LAB_FILE_NAME}"), lab_info.to_string())?;

    let mgmt_net = SherpaNetwork {
        v4: NetworkV4 {
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
    let dns = default_dns(&lab_net)?;

    println!("Creating network: {SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}");
    // Libvirt networks
    let management_network = NatNetwork {
        network_name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
        bridge_name: format!("{SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX}-{lab_id}"),
        ipv4_address: gateway_ip,
        ipv4_netmask: lab_net.netmask(),
    };
    management_network.create(&qemu_conn)?;

    println!("Creating network: {SHERPA_ISOLATED_NETWORK_NAME}-{lab_id}");
    let isolated_network = IsolatedNetwork {
        network_name: format!("{SHERPA_ISOLATED_NETWORK_NAME}-{lab_id}"),
        bridge_name: format!("{SHERPA_ISOLATED_NETWORK_BRIDGE_PREFIX}-{lab_id}"),
    };
    isolated_network.create(&qemu_conn)?;

    // Docker Networks
    create_docker_bridge_network(
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
    term_msg_underline("Creating Point-to-Point Links");
    for (idx, link) in links_detailed.iter().enumerate() {
        let link_index = idx as u16 + 1;

        let node_a = lab_node_data
            .iter()
            .find(|n| n.name == link.node_a)
            .ok_or_else(|| anyhow!("Node not found: {}", link.node_a))?;

        let node_b = lab_node_data
            .iter()
            .find(|n| n.name == link.node_b)
            .ok_or_else(|| anyhow!("Node not found: {}", link.node_b))?;

        // Generate unique, names must fit within Linux interface name limits (15 chars)
        let bridge_a = format!("{}a{}-{}", BRIDGE_PREFIX, link_index, lab_id);
        let bridge_b = format!("{}b{}-{}", BRIDGE_PREFIX, link_index, lab_id);
        let veth_a = format!("{}a{}-{}", VETH_PREFIX, link_index, lab_id);
        let veth_b = format!("{}b{}-{}", VETH_PREFIX, link_index, lab_id);

        // Create the link in the database
        let _db_link = create_link(
            &db,
            link_index,
            BridgeKind::P2pBridge,
            node_a.record.id.clone().ok_or_else(|| anyhow!("Node A has no ID"))?,
            node_b.record.id.clone().ok_or_else(|| anyhow!("Node B has no ID"))?,
            link.int_a.clone(),
            link.int_b.clone(),
            bridge_a.clone(),
            bridge_b.clone(),
            veth_a.clone(),
            veth_b.clone(),
            lab_record.id.clone().ok_or_else(|| anyhow!("Lab has no ID"))?,
        ).await?;

        // Store link data for later use (still needed for infrastructure setup)
        let link_data = LabLinkData {
            index: link_index,
            kind: BridgeKind::P2pBridge,
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
        create_bridge(
            &bridge_a,
            &format!("{}-bridge-{}::{}", lab_id, link.node_a, link.int_a),
        )
        .await?;
        create_bridge(
            &bridge_b,
            &format!("{}-bridge-{}::{}", lab_id, link.node_b, link.int_b),
        )
        .await?;
        create_veth_pair(
            &veth_a,
            &veth_b,
            &format!("{}-veth-{}::{}", lab_id, link.node_a, link.int_a),
            &format!("{}-veth-{}::{}", lab_id, link.node_b, link.int_b),
        )
        .await?;
        enslave_to_bridge(&veth_a, &bridge_a).await?;
        enslave_to_bridge(&veth_b, &bridge_b).await?;
    }

    term_msg_underline("ZTP");
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
        let node_idx = node_id_map
            .get(&node.name)
            .ok_or_else(|| anyhow::anyhow!("Node not found in node ID map: {}", node.name))?;

        let node_ip_idx = 10 + node_idx.to_owned() as u32;

        // generate the template
        println!("Creating container config: {}", node.name);
        let user = sherpa_user.clone();
        let dir = format!("{}/{}", lab_dir, node.name);

        node.ipv4_address = Some(get_ipv4_addr(&mgmt_net.v4.prefix, node_ip_idx)?);

        match node.model {
            NodeModel::AristaCeos => {
                let arista_template = AristaCeosZtpTemplate {
                    hostname: node.name.clone(),
                    user: user.clone(),
                    dns: dns.clone(),
                    mgmt_ipv4_address: node.ipv4_address,
                    mgmt_ipv4: mgmt_net.v4.clone(),
                };
                let rendered_template = arista_template.render()?;
                let ztp_config = format!("{dir}/{}.conf", node.name);
                let ztp_volume = VolumeMount {
                    src: ztp_config.clone(),
                    dst: ARISTA_CEOS_ZTP_VOLUME_MOUNT.to_string(),
                };
                create_dir(&dir)?;
                create_file(&ztp_config, rendered_template)?;

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
            NodeModel::NokiaSrlinux => {
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
            NodeModel::SurrealDb => {
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
        let node_idx = node_id_map
            .get(&node.name)
            .ok_or_else(|| anyhow::anyhow!("Node not found in node ID map: {}", node.name))?;

        let node_ip_idx = 10 + node_idx.to_owned() as u32;

        let node_model = config
            .node_config
            .iter()
            .find(|d| d.model == node.model)
            .ok_or_else(|| anyhow::anyhow!("Node model not found: {}", node.model))?;

        let mut disks: Vec<NodeDisk> = vec![];
        let node_name = format!("{}-{}", node.name, lab_id);

        let hdd_bus = node_model.hdd_bus.clone();
        let cdrom_bus = node_model.cdrom_bus.clone();

        let mac_address = random_mac(KVM_OUI);
        ztp_records.push(ZtpRecord {
            node_name: node.name.clone().to_owned(),
            config_file: format!("{}.conf", &node.name),
            ipv4_address: get_ipv4_addr(&mgmt_net.v4.prefix, node_ip_idx)?,
            mac_address: mac_address.to_string(),
            ztp_method: node_model.ztp_method.clone(),
            ssh_port: 22,
        });

        let mut interfaces: Vec<Interface> = vec![];

        // Management Interfaces
        if node_model.dedicated_management_interface {
            interfaces.push(Interface {
                name: dasher(&node_model.management_interface.to_string()),
                num: 0,
                mtu: node_model.interface_mtu,
                mac_address: mac_address.to_string(),
                connection_type: ConnectionTypes::Management,
                interface_connection: None,
            });
        }

        // Reserved Interfaces
        if node_model.reserved_interface_count > 0 {
            for i in node_model.first_interface_index..node_model.reserved_interface_count {
                interfaces.push(Interface {
                    name: format!("int{i}"),
                    num: i,
                    mtu: node_model.interface_mtu,
                    mac_address: random_mac(KVM_OUI),
                    connection_type: ConnectionTypes::Reserved,
                    interface_connection: None,
                });
            }
        }

        let end_iface_index = if node_model.first_interface_index == 0 {
            node_model.interface_count - 1
        } else {
            node_model.interface_count
        };
        for i in node_model.first_interface_index..=end_iface_index {
            // When node does not have a dedicated management interface the first_interface_index
            // Is assigned as a management interface
            if !node_model.dedicated_management_interface && i == node_model.first_interface_index {
                interfaces.push(Interface {
                    name: dasher(&node_model.management_interface.to_string()),
                    num: node_model.first_interface_index,
                    mtu: node_model.interface_mtu,
                    mac_address: mac_address.to_string(),
                    connection_type: ConnectionTypes::Management,
                    interface_connection: None,
                });
                continue;
            }
            // node to node links
            if !links_detailed.is_empty() {
                let mut p2p_connection = false;
                for l in links_detailed.iter() {
                    // node is source in manifest
                    if l.node_a == node.name && i == l.int_a_idx {
                        let source_id = node_id_map.get(&l.node_b).ok_or_else(|| {
                            anyhow::anyhow!("Connection dev_b not found: {}", l.node_b)
                        })?;
                        let local_id = node_id_map.get(&node.name).unwrap().to_owned(); // should never error
                        let interface_connection = InterfaceConnection {
                            local_id,
                            local_port: id_to_port(i),
                            local_loopback: get_ip(local_id as u8).to_string(),
                            source_id: source_id.to_owned(),
                            source_port: id_to_port(l.int_b_idx),
                            source_loopback: get_ip(source_id.to_owned() as u8).to_string(),
                        };
                        // interfaces.push(Interface {
                        //     name: dasher(&l.int_a),
                        //     num: i,
                        //     mtu: node_model.interface_mtu,
                        //     mac_address: random_mac(KVM_OUI),
                        //     connection_type: ConnectionTypes::Peer,
                        //     interface_connection: Some(interface_connection),
                        // });
                        interfaces.push(Interface {
                            name: format!("{}a{}-{}", BRIDGE_PREFIX, l.link_idx, lab_id),
                            num: i,
                            mtu: node_model.interface_mtu,
                            mac_address: random_mac(KVM_OUI),
                            connection_type: ConnectionTypes::PeerBridge,
                            interface_connection: Some(interface_connection),
                        });
                        p2p_connection = true;
                        break;
                    // node is destination in manifest
                    } else if l.node_b == node.name && i == l.int_b_idx {
                        let source_id = node_id_map.get(&l.node_a).ok_or_else(|| {
                            anyhow::anyhow!("Connection dev_a not found: {}", l.node_a)
                        })?;
                        let local_id = node_id_map.get(&node.name).unwrap().to_owned(); // should never error
                        let interface_connection = InterfaceConnection {
                            local_id,
                            local_port: id_to_port(i),
                            local_loopback: get_ip(local_id as u8).to_string(),
                            source_id: source_id.to_owned(),
                            source_port: id_to_port(l.int_a_idx),
                            source_loopback: get_ip(source_id.to_owned() as u8).to_string(),
                        };
                        // interfaces.push(Interface {
                        //     name: dasher(&l.int_b),
                        //     num: i,
                        //     mtu: node_model.interface_mtu,
                        //     mac_address: random_mac(KVM_OUI),
                        //     connection_type: ConnectionTypes::Peer,
                        //     interface_connection: Some(interface_connection),
                        // });
                        interfaces.push(Interface {
                            name: format!("{}b{}-{}", BRIDGE_PREFIX, l.link_idx, lab_id),
                            num: i,
                            mtu: node_model.interface_mtu,
                            mac_address: random_mac(KVM_OUI),
                            connection_type: ConnectionTypes::PeerBridge,
                            interface_connection: Some(interface_connection),
                        });
                        p2p_connection = true;
                        break;
                    }
                }
                if !p2p_connection {
                    // Interface not defined in manifest so disable.
                    interfaces.push(Interface {
                        name: dasher(&interface_from_idx(&node.model, i)?),
                        num: i,
                        mtu: node_model.interface_mtu,
                        mac_address: random_mac(KVM_OUI),
                        connection_type: ConnectionTypes::Disabled,
                        interface_connection: None,
                    })
                }
            } else {
                interfaces.push(Interface {
                    name: dasher(&interface_from_idx(&node.model, i)?),
                    num: i,
                    mtu: node_model.interface_mtu,
                    mac_address: random_mac(KVM_OUI),
                    connection_type: ConnectionTypes::Disabled,
                    interface_connection: None,
                })
            }
        }

        // Only Virtual machines have a boot disk to clone.
        let vm_boot_disk = match node_model.kind {
            NodeKind::VirtualMachine => {
                let src_boot_disk = format!(
                    "{}/{}/{}/virtioa.qcow2",
                    sherpa.images_dir, node_model.model, node_model.version
                );
                let dst_boot_disk = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name}-hdd.qcow2");

                clone_disks.push(CloneDisk {
                    src: src_boot_disk.clone(),
                    dst: dst_boot_disk.clone(),
                });

                Some(dst_boot_disk)
            }
            _ => None,
        };

        // CDROM ISO
        let (mut src_cdrom_iso, mut dst_cdrom_iso) = match &node_model.cdrom {
            Some(src_iso) => {
                let src = format!(
                    "{}/{}/{}/{}",
                    sherpa.images_dir, node_model.model, node_model.version, src_iso
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

        if node_model.ztp_enable {
            // vm_nodes.push(node.clone());
            // TODO: Update this to use the assigned IP if
            // an IP is not user defined.
            let node_ipv4_address = ztp_records
                .iter()
                .find(|r| r.node_name == node.name)
                .map(|r| r.ipv4_address);
            match node_model.ztp_method {
                ZtpMethod::CloudInit => {
                    term_msg_underline("Creating Cloud-Init disks");
                    // generate the template
                    println!("Creating Cloud-Init config {}", node.name);
                    let dir = format!("{lab_dir}/{node_name}");
                    let mut cloud_init_user = CloudInitUser::sherpa()?;

                    match node.model {
                        NodeModel::CentosLinux
                        | NodeModel::AlmaLinux
                        | NodeModel::RockyLinux
                        | NodeModel::FedoraLinux
                        | NodeModel::OpensuseLinux
                        | NodeModel::RedhatLinux
                        | NodeModel::SuseLinux
                        | NodeModel::UbuntuLinux
                        | NodeModel::FreeBsd
                        | NodeModel::OpenBsd => {
                            let (admin_group, shell) = match node_model.os_variant {
                                OsVariant::Bsd => ("wheel".to_string(), "/bin/sh".to_string()),
                                _ => ("sudo".to_string(), "/bin/bash".to_string()),
                            };
                            cloud_init_user.groups = vec![admin_group];
                            cloud_init_user.shell = shell;

                            let cloud_init_config = CloudInitConfig {
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

                            create_dir(&dir)?;
                            create_file(&user_data, user_data_config)?;
                            create_file(&meta_data, "".to_string())?;

                            if node_ipv4_address.is_some() {
                                let ztp_interface = CloudInitNetwork::ztp_interface(
                                    // This should always be Some
                                    node_ipv4_address.unwrap(),
                                    mac_address,
                                    mgmt_net.v4.clone(),
                                );
                                let cloud_network_config = ztp_interface.to_string()?;
                                create_file(&network_config, cloud_network_config)?;
                            }

                            create_ztp_iso(&format!("{}/{}", dir, ZTP_ISO), dir)?
                        }

                        NodeModel::AlpineLinux => {
                            let meta_data = MetaDataConfig {
                                instance_id: format!("iid-{}", node.name.clone(),),
                                local_hostname: format!(
                                    "{}.{}",
                                    node.name.clone(),
                                    SHERPA_DOMAIN_NAME
                                ),
                            };
                            cloud_init_user.shell = "/bin/sh".to_string();
                            cloud_init_user.groups = vec!["wheel".to_string()];
                            let cloud_init_config = CloudInitConfig {
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

                            create_dir(&dir)?;
                            create_file(&user_data, user_data_config)?;
                            create_file(&meta_data, meta_data_config)?;

                            if node_ipv4_address.is_some() {
                                let ztp_interface = CloudInitNetwork::ztp_interface(
                                    // This should always be Some
                                    node_ipv4_address.unwrap(),
                                    mac_address,
                                    mgmt_net.v4.clone(),
                                );
                                let cloud_network_config = ztp_interface.to_string()?;
                                create_file(&network_config, cloud_network_config)?;
                            }

                            create_ztp_iso(&format!("{}/{}", dir, ZTP_ISO), dir)?
                        }
                        _ => {
                            anyhow::bail!(
                                "CDROM ZTP method not supported for {}",
                                node_model.model
                            );
                        }
                    }
                    src_cdrom_iso = Some(format!("{lab_dir}/{node_name}/{ZTP_ISO}"));
                    dst_cdrom_iso = Some(format!("{SHERPA_STORAGE_POOL_PATH}/{node_name}.iso"));
                }
                ZtpMethod::Cdrom => {
                    term_msg_underline("Creating ZTP disks");
                    // generate the template
                    println!("Creating ZTP config {}", node.name);
                    let mut user = sherpa_user.clone();
                    let dir = format!("{lab_dir}/{node_name}");

                    match node.model {
                        NodeModel::CiscoCsr1000v
                        | NodeModel::CiscoCat8000v
                        | NodeModel::CiscoCat9000v => {
                            let license_boot_command = if node.model == NodeModel::CiscoCat8000v {
                                Some(
                                    "license boot level network-premier addon dna-premier"
                                        .to_string(),
                                )
                            } else if node.model == NodeModel::CiscoCat9000v {
                                Some(
                                    "license boot level network-advantage addon dna-advantage"
                                        .to_string(),
                                )
                            } else {
                                None
                            };

                            let key_hash = pub_ssh_key_to_md5_hash(&user.ssh_public_key.key)?;
                            user.ssh_public_key.key = key_hash;
                            let t = CiscoIosXeZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                mgmt_interface: node_model.management_interface.to_string(),
                                dns: dns.clone(),
                                license_boot_command,
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let c = CISCO_IOSXE_ZTP_CONFIG.replace("-", "_");
                            let ztp_config = format!("{dir}/{c}");
                            create_dir(&dir)?;
                            create_file(&ztp_config, rendered_template)?;
                            create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?
                        }
                        NodeModel::CiscoAsav => {
                            let key_hash = pub_ssh_key_to_sha256_hash(&user.ssh_public_key.key)?;
                            user.ssh_public_key.key = key_hash;
                            let t = CiscoAsavZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                dns: dns.clone(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{CISCO_ASAV_ZTP_CONFIG}");
                            create_dir(&dir)?;
                            create_file(&ztp_config, rendered_template)?;
                            create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?
                        }
                        NodeModel::CiscoNexus9300v => {
                            let t = CiscoNxosZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                dns: dns.clone(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{CISCO_NXOS_ZTP_CONFIG}");
                            create_dir(&dir)?;
                            create_file(&ztp_config, rendered_template)?;
                            create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?
                        }
                        NodeModel::CiscoIosxrv9000 => {
                            let t = CiscoIosxrZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                dns: dns.clone(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{CISCO_IOSXR_ZTP_CONFIG}");
                            create_dir(&dir)?;
                            create_file(&ztp_config, rendered_template)?;
                            create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?
                        }
                        NodeModel::CiscoFtdv => {
                            let t = CiscoFtdvZtpTemplate {
                                eula: "accept".to_string(),
                                hostname: node.name.clone(),
                                admin_password: SHERPA_PASSWORD.to_string(),
                                dns1: Some(mgmt_net.v4.boot_server),
                                ipv4_mode: Some(CiscoFxosIpMode::Manual),
                                ipv4_addr: node_ipv4_address,
                                ipv4_gw: Some(mgmt_net.v4.first),
                                ipv4_mask: Some(mgmt_net.v4.subnet_mask),
                                manage_locally: true,
                                ..Default::default()
                            };
                            let rendered_template = serde_json::to_string(&t)?;
                            let ztp_config = format!("{dir}/{CISCO_FTDV_ZTP_CONFIG}");
                            create_dir(&dir)?;
                            create_file(&ztp_config, rendered_template)?;
                            create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?
                        }
                        NodeModel::JuniperVsrxv3
                        | NodeModel::JuniperVrouter
                        | NodeModel::JuniperVswitch => {
                            let t = JunipervJunosZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                mgmt_interface: node_model.management_interface.to_string(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{JUNIPER_ZTP_CONFIG}");
                            create_dir(&dir)?;
                            create_file(&ztp_config, rendered_template)?;
                            create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?
                        }
                        _ => {
                            anyhow::bail!(
                                "CDROM ZTP method not supported for {}",
                                node_model.model
                            );
                        }
                    };
                    src_cdrom_iso = Some(format!("{lab_dir}/{node_name}/{ZTP_ISO}"));
                    dst_cdrom_iso = Some(format!("{SHERPA_STORAGE_POOL_PATH}/{node_name}-cfg.iso"));
                }
                ZtpMethod::Tftp => {
                    // generate the template
                    println!("Creating ZTP config {}", node.name);
                    let user = sherpa_user.clone();
                    let dir = format!("{lab_dir}/{ZTP_DIR}/{TFTP_DIR}");

                    match node.model {
                        NodeModel::AristaVeos => {
                            let arista_template = AristaVeosZtpTemplate {
                                hostname: node.name.clone(),
                                user: user.clone(),
                                dns: dns.clone(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = arista_template.render()?;
                            let ztp_config = format!("{dir}/{}.conf", node.name);
                            create_dir(&dir)?;
                            create_file(&ztp_config, rendered_template)?;
                        }
                        NodeModel::ArubaAoscx => {
                            let aruba_template = ArubaAoscxTemplate {
                                hostname: node.name.clone(),
                                user: user.clone(),
                                dns: dns.clone(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let aruba_rendered_template = aruba_template.render()?;
                            let ztp_config = format!("{dir}/{}.conf", node.name);
                            create_dir(&dir)?;
                            create_file(&ztp_config, aruba_rendered_template)?;
                        }
                        NodeModel::JuniperVevolved => {
                            let juniper_template = JunipervJunosZtpTemplate {
                                hostname: node.name.clone(),
                                user: sherpa_user.clone(),
                                mgmt_interface: node_model.management_interface.to_string(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let juniper_rendered_template = juniper_template.render()?;
                            let ztp_config = format!("{dir}/{}.conf", node.name);
                            create_dir(&dir)?;
                            create_file(&ztp_config, juniper_rendered_template)?;
                        }
                        _ => {
                            anyhow::bail!("Tftp ZTP method not supported for {}", node_model.model);
                        }
                    }
                }
                ZtpMethod::Http => {
                    // generate the template
                    println!("Creating ZTP config {}", node.name);
                    let _user = sherpa_user.clone();
                    let dir = format!("{lab_dir}/{ZTP_DIR}/{NODE_CONFIGS_DIR}");

                    match node.model {
                        NodeModel::SonicLinux => {
                            let sonic_ztp_file_map =
                                SonicLinuxZtp::file_map(&node.name, &mgmt_net.v4.boot_server);

                            let ztp_init = format!("{dir}/{}.conf", &node.name);
                            let sonic_ztp = SonicLinuxZtp {
                                hostname: node.name.clone(),
                                mgmt_ipv4: mgmt_net.v4.clone(),
                                mgmt_ipv4_address: node_ipv4_address,
                            };
                            let ztp_config = format!("{dir}/{}_config_db.json", &node.name);
                            create_dir(&dir)?;
                            create_file(&ztp_init, sonic_ztp_file_map)?;
                            create_file(&ztp_config, sonic_ztp.config())?;
                        }
                        _ => {
                            anyhow::bail!("HTTP ZTP method not supported for {}", node_model.model);
                        }
                    }
                }
                ZtpMethod::Disk => {
                    println!("Creating ZTP config {}", node.name);
                    let mut user = sherpa_user.clone();

                    let dir = format!("{lab_dir}/{node_name}");
                    match node.model {
                        NodeModel::CiscoIosv => {
                            let key_hash = pub_ssh_key_to_md5_hash(&user.ssh_public_key.key)?;
                            user.ssh_public_key.key = key_hash;
                            let t = CiscoIosvZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                mgmt_interface: node_model.management_interface.to_string(),
                                dns: dns.clone(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let c = CISCO_IOSV_ZTP_CONFIG;
                            let ztp_config = format!("{dir}/{c}");
                            create_dir(&dir)?;
                            create_file(&ztp_config, rendered_template)?;
                            // clone disk
                            let src_disk = format!(
                                "{}/{}/{}",
                                &sherpa.images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_IOSV
                            );
                            let dst_disk = format!("{dir}/{node_name}-cfg.img");

                            // Create a copy of the disk base image
                            copy_file(&src_disk, &dst_disk)?;
                            // copy file to disk disk
                            copy_to_dos_image(&ztp_config, &dst_disk, "/")?;

                            src_config_disk = Some(dst_disk.to_owned());
                            dst_config_disk =
                                Some(format!("{SHERPA_STORAGE_POOL_PATH}/{node_name}-cfg.img"));
                        }
                        NodeModel::CiscoIosvl2 => {
                            let key_hash = pub_ssh_key_to_md5_hash(&user.ssh_public_key.key)?;
                            user.ssh_public_key.key = key_hash;
                            let t = CiscoIosvl2ZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                mgmt_interface: node_model.management_interface.to_string(),
                                dns: dns.clone(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let c = CISCO_IOSV_ZTP_CONFIG;
                            let ztp_config = format!("{dir}/{c}");
                            create_dir(&dir)?;
                            create_file(&ztp_config, rendered_template)?;
                            // clone disk
                            let src_disk = format!(
                                "{}/{}/{}",
                                &sherpa.images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_IOSV
                            );
                            let dst_disk = format!("{dir}/{node_name}-cfg.img");

                            // Create a copy of the hdd base image
                            copy_file(&src_disk, &dst_disk)?;
                            // copy file to hdd disk
                            copy_to_dos_image(&ztp_config, &dst_disk, "/")?;

                            src_config_disk = Some(dst_disk.to_owned());
                            dst_config_disk =
                                Some(format!("{SHERPA_STORAGE_POOL_PATH}/{node_name}-cfg.img"));
                        }
                        NodeModel::CiscoIse => {
                            let t = CiscoIseZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                dns: dns.clone(),
                                mgmt_ipv4_address: node_ipv4_address.ok_or_else(|| anyhow!("Cisco ISE node model requires an IPv4 management address. Node: {}", node.name))?,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{CISCO_ISE_ZTP_CONFIG}");
                            create_dir(&dir)?;
                            create_file(&ztp_config, rendered_template)?;

                            // clone disk
                            let src_disk = format!(
                                "{}/{}/{}",
                                &sherpa.images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_ISE
                            );
                            let dst_disk = format!("{dir}/{node_name}-cfg.img");

                            // Create a copy of the hdd base image
                            copy_file(&src_disk, &dst_disk)?;
                            // copy file to hdd disk
                            copy_to_ext4_image(vec![&ztp_config], &dst_disk, "/")?;

                            src_config_disk = Some(dst_disk.to_owned());
                            dst_config_disk =
                                Some(format!("{SHERPA_STORAGE_POOL_PATH}/{node_name}-cfg.img"));
                        }
                        _ => {
                            anyhow::bail!("Disk ZTP method not supported for {}", node_model.model);
                        }
                    }
                }
                ZtpMethod::Usb => {
                    // generate the template
                    println!("Creating ZTP config {}", node.name);
                    let user = sherpa_user.clone();
                    let dir = format!("{lab_dir}/{node_name}");

                    match node_model.model {
                        NodeModel::CumulusLinux => {
                            let t = CumulusLinuxZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                dns: dns.clone(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{CUMULUS_ZTP}");
                            create_dir(&dir)?;
                            create_file(&ztp_config, rendered_template)?;
                            // clone USB disk
                            let src_usb = format!(
                                "{}/{}/{}",
                                &sherpa.images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_FAT32
                            );

                            let dst_usb = format!("{dir}/cfg.img");

                            // Create a copy of the usb base image
                            copy_file(&src_usb, &dst_usb)?;
                            // copy file to USB disk
                            copy_to_dos_image(&ztp_config, &dst_usb, "/")?;

                            src_usb_disk = Some(dst_usb.to_owned());
                            dst_usb_disk =
                                Some(format!("{SHERPA_STORAGE_POOL_PATH}/{node_name}-cfg.img"));
                        }
                        NodeModel::JuniperVevolved => {
                            let t = JunipervJunosZtpTemplate {
                                hostname: node.name.clone(),
                                user,
                                mgmt_interface: node_model.management_interface.to_string(),
                                mgmt_ipv4_address: node_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{JUNIPER_ZTP_CONFIG}");
                            let ztp_config_tgz = format!("{dir}/{JUNIPER_ZTP_CONFIG_TGZ}");

                            create_dir(&dir)?;
                            create_file(&ztp_config, rendered_template)?;
                            // clone USB disk
                            let src_usb = format!(
                                "{}/{}/{}",
                                &sherpa.images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_JUNOS
                            );
                            let dst_usb = format!("{dir}/cfg.img");

                            // Create a copy of the usb base image
                            copy_file(&src_usb, &dst_usb)?;

                            // Create tar.gz config file
                            create_config_archive(&ztp_config, &ztp_config_tgz)?;

                            // copy file to USB disk
                            copy_to_dos_image(&ztp_config_tgz, &dst_usb, "/")?;
                            // copy_to_dos_image(&ztp_config, &dst_usb, "/")?;

                            src_usb_disk = Some(dst_usb.to_owned());
                            dst_usb_disk =
                                Some(format!("{SHERPA_STORAGE_POOL_PATH}/{node_name}-cfg.img"));
                        }
                        _ => {
                            anyhow::bail!("USB ZTP method not supported for {}", node_model.model);
                        }
                    }
                }
                ZtpMethod::Ignition => {
                    term_msg_underline("Creating ZTP disks");
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
                            let ssh_key = get_ssh_public_key(&file.source)?;
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

                    let ignition_user = IgnitionUser {
                        name: user.username.clone(),
                        password_hash: SHERPA_PASSWORD_HASH.to_owned(),
                        ssh_authorized_keys: authorized_keys,
                        groups: vec!["wheel".to_owned(), "docker".to_owned()],
                    };
                    let hostname_file = IgnitionFile {
                        path: "/etc/hostname".to_owned(),
                        mode: 644,
                        contents: IgnitionFileContents::new(&format!("data:,{dev_name}",)),
                        ..Default::default()
                    };
                    // files
                    let disable_update = IgnitionFile::disable_updates();
                    let sudo_config_base64 =
                        base64_encode(&format!("{SHERPA_USERNAME} ALL=(ALL) NOPASSWD: ALL"));
                    let sudo_config_file = IgnitionFile {
                        path: format!("/etc/sudoers.d/{SHERPA_USERNAME}"),
                        mode: 440,
                        contents: IgnitionFileContents::new(&format!(
                            "data:;base64,{sudo_config_base64}"
                        )),
                        ..Default::default()
                    };
                    let manifest_text_files: Vec<IgnitionFile> = node
                        .text_files
                        .iter() // Iterator over Option<Vec<File>>
                        .flatten() // Flattens Option<Vec<File>> to individual &File items
                        .map(|file| {
                            let encoded_file = base64_encode_file(&file.source)?;

                            Ok(IgnitionFile {
                                path: file.destination.clone(),
                                mode: file.permissions,
                                overwrite: None,
                                contents: IgnitionFileContents::new(&format!(
                                    "data:;base64,{encoded_file}"
                                )),
                                user: Some(IgnitionFileParams {
                                    name: file.user.clone(),
                                }),
                                group: Some(IgnitionFileParams {
                                    name: file.group.clone(),
                                }),
                            })
                        })
                        .collect::<Result<Vec<IgnitionFile>>>()?;

                    let manifest_binary_disk_files = node.binary_files.clone().unwrap_or(vec![]);

                    let manifest_systemd_units: Vec<IgnitionUnit> = node
                        .systemd_units
                        .iter() // Iterator over Option<Vec<File>>
                        .flatten() // Flattens Option<Vec<File>> to individual &File items
                        .map(|file| {
                            let file_contents = load_file(file.source.as_str())?;
                            Ok(IgnitionUnit {
                                name: file.name.clone(),
                                enabled: Some(file.enabled),
                                contents: Some(file_contents),
                                ..Default::default()
                            })
                        })
                        .collect::<Result<Vec<IgnitionUnit>>>()?;

                    match node.model {
                        NodeModel::FlatcarLinux => {
                            let mut units = vec![];
                            units.push(IgnitionUnit::mount_container_disk());
                            units.extend(manifest_systemd_units);

                            let container_disk = IgnitionFileSystem::default();

                            let mut files = vec![sudo_config_file, hostname_file, disable_update];
                            files.extend(manifest_text_files);

                            if node_ipv4_address.is_some() {
                                files.push(IgnitionFile::ztp_interface(
                                    // This should always be Some
                                    node_ipv4_address.unwrap(),
                                    mgmt_net.v4.clone(),
                                )?);
                            }

                            let ignition_config = IgnitionConfig::new(
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

                            create_dir(&dir)?;
                            create_file(&src_ztp_file, flatcar_config)?;

                            // Copy a blank disk to to .tmp directory
                            let src_data_disk = format!(
                                "{}/{}/{}",
                                &sherpa.images_dir,
                                SHERPA_BLANK_DISK_DIR,
                                SHERPA_BLANK_DISK_EXT4_500MB
                            );
                            let dst_disk = format!("{dir}/{node_name}-{CONTAINER_DISK_NAME}");

                            copy_file(&src_data_disk, &dst_disk)?;

                            let disk_files: Vec<&str> = manifest_binary_disk_files
                                .iter()
                                .map(|x| x.source.as_str())
                                .collect();

                            // Copy to container image into the container disk
                            if !disk_files.is_empty() {
                                copy_to_ext4_image(disk_files, &dst_disk, "/")?;
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
                                node_model.model
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        // ISO
        if let (Some(src_cdrom_iso), Some(dst_cdrom_iso)) = (src_cdrom_iso, dst_cdrom_iso) {
            clone_disks.push(CloneDisk {
                // These should always have a value.
                src: src_cdrom_iso,
                dst: dst_cdrom_iso.clone(),
            });
            disks.push(NodeDisk {
                disk_device: DiskDevices::Cdrom,
                driver_name: DiskDrivers::Qemu,
                driver_format: DiskFormats::Raw,
                src_file: dst_cdrom_iso.clone(),
                target_dev: DiskTargets::target(&cdrom_bus, disks.len() as u8)?,
                target_bus: cdrom_bus.clone(),
            });
        }

        // Hdd
        if let Some(vm_boot_disk) = vm_boot_disk {
            disks.push(NodeDisk {
                disk_device: DiskDevices::File,
                driver_name: DiskDrivers::Qemu,
                driver_format: DiskFormats::Qcow2,
                src_file: vm_boot_disk.clone(),
                target_dev: DiskTargets::target(&hdd_bus, disks.len() as u8)?,
                target_bus: hdd_bus.clone(),
            });
        }

        // Data Disk
        if let (Some(src_config_disk), Some(dst_config_disk)) = (src_config_disk, dst_config_disk) {
            clone_disks.push(CloneDisk {
                src: src_config_disk,
                dst: dst_config_disk.clone(),
            });
            disks.push(NodeDisk {
                disk_device: DiskDevices::File,
                driver_name: DiskDrivers::Qemu,
                driver_format: DiskFormats::Raw,
                src_file: dst_config_disk.clone(),
                target_dev: DiskTargets::target(&hdd_bus, disks.len() as u8)?,
                target_bus: hdd_bus.clone(),
            });
        }

        // USB
        if let (Some(src_usb_disk), Some(dst_usb_disk)) = (src_usb_disk, dst_usb_disk) {
            clone_disks.push(CloneDisk {
                src: src_usb_disk,
                dst: dst_usb_disk.clone(),
            });
            disks.push(NodeDisk {
                disk_device: DiskDevices::File,
                driver_name: DiskDrivers::Qemu,
                driver_format: DiskFormats::Raw,
                src_file: dst_usb_disk.clone(),
                target_dev: DiskTargets::target(&DiskBuses::Usb, disks.len() as u8)?,
                target_bus: DiskBuses::Usb,
            });
        }

        // Ignition
        if let (Some(src_ignition_disk), Some(dst_ignition_disk)) =
            (src_ignition_disk, dst_ignition_disk.clone())
        {
            clone_disks.push(CloneDisk {
                src: src_ignition_disk,
                dst: dst_ignition_disk.clone(),
            });
            disks.push(NodeDisk {
                disk_device: DiskDevices::File,
                driver_name: DiskDrivers::Qemu,
                driver_format: DiskFormats::Raw,
                src_file: dst_ignition_disk.clone(),
                target_dev: DiskTargets::target(&DiskBuses::Sata, disks.len() as u8)?,
                target_bus: DiskBuses::Sata,
            });
        }

        let qemu_commands = match node_model.model {
            NodeModel::JuniperVrouter => QemuCommand::juniper_vrouter(),
            NodeModel::JuniperVswitch => QemuCommand::juniper_vswitch(),
            NodeModel::JuniperVevolved => QemuCommand::juniper_vevolved(),
            NodeModel::FlatcarLinux => {
                if let Some(dst_ignition_disk) = dst_ignition_disk {
                    QemuCommand::ignition_config(&dst_ignition_disk)
                } else {
                    vec![]
                }
            }
            _ => {
                vec![]
            }
        };

        let node_id = node_id_map.get(&node.name).unwrap().to_owned(); // should never error

        if node_model.kind == NodeKind::VirtualMachine {
            let domain = DomainTemplate {
                qemu_bin: config.qemu_bin.clone(),
                name: node_name,
                memory: node.memory.unwrap_or(node_model.memory),
                cpu_architecture: node_model.cpu_architecture.clone(),
                cpu_model: node_model.cpu_model.clone(),
                machine_type: node_model.machine_type.clone(),
                cpu_count: node.cpu_count.unwrap_or(node_model.cpu_count),
                vmx_enabled: node_model.vmx_enabled,
                bios: node_model.bios.clone(),
                disks,
                interfaces,
                interface_type: node_model.interface_type.clone(),
                loopback_ipv4: get_ip(node_id as u8).to_string(),
                telnet_port: TELNET_PORT,
                qemu_commands,
                lab_id: lab_id.to_string(),
            };
            domains.push(domain);
        }
    }

    create_ztp_files(&mgmt_net, &sherpa_user, &lab_id, &ztp_records)?;
    create_boot_containers(&docker_conn, &mgmt_net, lab_id).await?;

    // Clone disks in parallel
    term_msg_underline("Cloning Disks");
    let disk_handles: Vec<_> = clone_disks
        .into_iter()
        .map(|disk| {
            let qemu_conn = Arc::clone(&qemu_conn);
            thread::spawn(move || -> Result<()> {
                println!("Cloning disk \n  from: {} \n    to: {}", disk.src, disk.dst);
                clone_disk(&qemu_conn, &disk.src, &disk.dst).with_context(|| {
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
            .map_err(|e| anyhow::anyhow!("Error cloning disk: {:?}", e))??;
    }

    // Build domains in parallel
    term_msg_underline("Creating Node Configs");

    let vm_handles: Vec<_> = domains
        .into_iter()
        .map(|domain| {
            let qemu_conn = Arc::clone(&qemu_conn);
            thread::spawn(move || -> Result<()> {
                let rendered_xml = domain
                    .render()
                    .with_context(|| format!("Failed to render XML for VM: {}", domain.name))?;

                println!("Creating VM: {}", domain.name);
                create_vm(&qemu_conn, &rendered_xml)
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
            .map_err(|e| anyhow::anyhow!("Error creating VM: {:?}", e))??;
    }

    if !ztp_records.is_empty() {
        if config.inventory_management.pyats {
            term_msg_underline("Creating PyATS Testbed File");
            let pyats_inventory = PyatsInventory::from_manifest(manifest, &config, &ztp_records)?;
            let pyats_yaml = pyats_inventory.to_yaml()?;
            create_file(&format!("{lab_dir}/testbed.yaml"), pyats_yaml)?;
        }

        term_msg_underline("Creating SSH Config File");
        let ssh_config_template = SshConfigTemplate {
            ztp_records: ztp_records.clone(),
        };
        let rendered_template = ssh_config_template.render()?;
        create_file(
            &format!("{lab_dir}/{SHERPA_SSH_CONFIG_FILE}"),
            rendered_template,
        )?;
    }

    // Check if VMs are ready
    term_msg_underline("Checking Node Readiness");
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

            let mgmt_net_attachment = ContainerNetworkAttachment {
                name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
                ipv4_address: mgmt_ipv4,
            };
            // let test_net_attachment = ContainerNetworkAttachment {
            //     name: "brb0-723035d2".to_string(),
            //     ipv4_address: None,
            // };

            run_container(
                //
                &docker_conn,
                &container_name,
                &container_image,
                env_vars,
                volumes,
                vec![],
                vec![mgmt_net_attachment], //test_net_attachment],
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
                match tcp_connect(&vm_data.ipv4_address.to_string(), SSH_PORT)? {
                    true => {
                        println!("{} - Ready", &vm.name);
                        connected_nodes.insert(vm.name.clone());
                        node_ip_map.push(NodeConnection {
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
