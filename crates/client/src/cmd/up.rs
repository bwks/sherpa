use super::boot_containers::{create_boot_containers, create_ztp_files};

use anyhow::{Context, Result};
use askama::Template;

use container::{create_network, docker_connection};
use data::{
    CloneDisk, ConnectionTypes, DeviceConnection, DeviceDisk, DeviceKind, DeviceModels, DiskBuses,
    DiskDevices, DiskDrivers, DiskFormats, DiskTargets, Interface, InterfaceConnection, LabInfo,
    NetworkV4, OsVariants, QemuCommand, Sherpa, SherpaNetwork, ZtpMethods, ZtpRecord,
};
use konst::{
    CISCO_ASAV_ZTP_CONFIG, CISCO_IOSV_ZTP_CONFIG, CISCO_IOSXE_ZTP_CONFIG, CISCO_IOSXR_ZTP_CONFIG,
    CISCO_NXOS_ZTP_CONFIG, CLOUD_INIT_META_DATA, CLOUD_INIT_NETWORK_CONFIG, CLOUD_INIT_USER_DATA,
    CONTAINER_DISK_NAME, CUMULUS_ZTP, DEVICE_CONFIGS_DIR, JUNIPER_ZTP_CONFIG,
    JUNIPER_ZTP_CONFIG_TGZ, KVM_OUI, LAB_FILE_NAME, READINESS_SLEEP, READINESS_TIMEOUT,
    SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_EXT4_500MB, SHERPA_BLANK_DISK_FAT32,
    SHERPA_BLANK_DISK_IOSV, SHERPA_BLANK_DISK_JUNOS, SHERPA_DOMAIN_NAME,
    SHERPA_ISOLATED_NETWORK_BRIDGE_PREFIX, SHERPA_ISOLATED_NETWORK_NAME,
    SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX, SHERPA_MANAGEMENT_NETWORK_NAME, SHERPA_PASSWORD_HASH,
    SHERPA_SSH_CONFIG_FILE, SHERPA_STORAGE_POOL_PATH, SHERPA_USERNAME, SSH_PORT, SSH_PORT_ALT,
    TELNET_PORT, TEMP_DIR, TFTP_DIR, ZTP_DIR, ZTP_ISO, ZTP_JSON,
};
use libvirt::{IsolatedNetwork, NatNetwork, Qemu, clone_disk, create_vm};
use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::thread::sleep;
use std::time::{Duration, Instant};
use template::{
    AristaVeosZtpTemplate, ArubaAoscxTemplate, CiscoAsavZtpTemplate, CiscoIosXeZtpTemplate,
    CiscoIosvZtpTemplate, CiscoIosvl2ZtpTemplate, CiscoIosxrZtpTemplate, CiscoNxosZtpTemplate,
    CloudInitConfig, CloudInitNetwork, CloudInitUser, Contents as IgnitionFileContents,
    CumulusLinuxZtpTemplate, DomainTemplate, File as IgnitionFile,
    FileParams as IgnitionFileParams, FileSystem as IgnitionFileSystem, IgnitionConfig,
    JunipervJunosZtpTemplate, MetaDataConfig, PyatsInventory, SonicLinuxZtp, SshConfigTemplate,
    Unit as IgnitionUnit, User as IgnitionUser,
};
use topology::{LinkDetailed, LinkExpanded, Manifest, Node};
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
    config_file: &str,
    qemu: &Qemu,
    _lab_name: &str,
    lab_id: &str,
    manifest: &Manifest,
) -> Result<()> {
    // Setup
    let docker_conn = docker_connection()?;
    let qemu_conn = Arc::new(qemu.connect()?);
    let sherpa_user = sherpa_user()?;

    term_msg_surround(&format!("Building environment - {lab_id}"));

    println!("Loading config");
    let mut sherpa = sherpa.clone();

    sherpa.config_path = format!("{}/{}", sherpa.config_dir, config_file);
    let mut config = load_config(&sherpa.config_path)?;

    // TODO: RUN EXISTING LAB VALIDATORS

    term_msg_underline("Validating Manifest");
    let manifest_links = manifest.links.clone().unwrap_or_default();

    // links from manifest links
    let links = manifest_links
        .iter()
        .map(|x| x.expand())
        .collect::<Result<Vec<LinkExpanded>>>()?;

    let mut links_detailed = vec![];
    for link in links.iter() {
        let mut this_link = LinkDetailed::default();
        for device in manifest.nodes.iter() {
            let device_model = device.model.clone();
            if link.dev_a == device.name {
                let int_idx = interface_to_idx(&device_model, &link.int_a)?;
                this_link.dev_a = device.name.clone();
                this_link.dev_a_model = device_model;
                this_link.int_a = link.int_a.clone();
                this_link.int_a_idx = int_idx;
            } else if link.dev_b == device.name {
                let int_idx = interface_to_idx(&device_model, &link.int_b)?;
                this_link.dev_b = device.name.clone();
                this_link.dev_b_model = device_model;
                this_link.int_b = link.int_b.clone();
                this_link.int_b_idx = int_idx;
            }
        }
        links_detailed.push(this_link)
    }

    // Device Validators
    check_duplicate_device(&manifest.nodes)?;

    let mut ztp_records = vec![];

    for device in &manifest.nodes {
        let device_model = config
            .device_models
            .iter()
            .find(|d| d.name == device.model)
            .ok_or_else(|| anyhow::anyhow!("Device model not found: {}", device.model))?;

        if !device_model.dedicated_management_interface {
            check_mgmt_usage(
                &device.name,
                device_model.first_interface_index,
                &links_detailed,
            )?;
        }

        check_interface_bounds(
            &device.name,
            &device_model.name,
            device_model.first_interface_index,
            device_model.interface_count,
            &links_detailed,
        )?;
    }

    // Connection Validators
    if !links.is_empty() {
        check_duplicate_interface_link(&links_detailed)?;
        check_link_device(&manifest.nodes, &links)?;
    };

    println!("Manifest Ok");

    // Create Temp DIR
    create_dir(TEMP_DIR)?;

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
    create_file(&format!("{TEMP_DIR}/{LAB_FILE_NAME}"), lab_info.to_string())?;

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
    create_network(
        &docker_conn,
        &format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
        Some(lab_net.to_string()),
        &format!("{SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX}-{lab_id}"),
    )
    .await?;

    term_msg_underline("ZTP");
    if manifest.ztp_server.is_some() {
        config.ztp_server.enable = manifest.ztp_server.clone().unwrap().enable
    }
    if config.ztp_server.enable {
        println!("ZTP server is enabled in configuration")
    } else {
        for device in &manifest.nodes {
            if device.model.needs_ztp_server() {
                println!("ZTP server is required");
                config.ztp_server.enable = true
            }
        }
        if !config.ztp_server.enable {
            println!("ZTP server is not required")
        }
    }

    // Create a mapping of device name to device id.
    // Devices have an id based on their order in the list of devices
    // from the manifest file.
    let dev_id_map: HashMap<String, u8> = manifest
        .nodes
        .iter()
        .enumerate()
        .map(|(idx, device)| (device.name.clone(), idx as u8 + 1))
        .collect();

    let mut ztp_devices: Vec<&Node> = vec![];
    let mut copy_disks: Vec<CloneDisk> = vec![];
    let mut domains: Vec<DomainTemplate> = vec![];
    for device in &manifest.nodes {
        let mut disks: Vec<DeviceDisk> = vec![];
        let vm_name = format!("{}-{}", device.name, lab_id);

        let device_idx = dev_id_map
            .get(&device.name)
            .ok_or_else(|| anyhow::anyhow!("Device not found in device ID map: {}", device.name))?;

        let device_ip_idx = 10 + device_idx.to_owned() as u32;

        let device_model = config
            .device_models
            .iter()
            .find(|d| d.name == device.model)
            .ok_or_else(|| anyhow::anyhow!("Device model not found: {}", device.model))?;

        let hdd_bus = device_model.hdd_bus.clone();
        let cdrom_bus = device_model.cdrom_bus.clone();

        let mac_address = random_mac(KVM_OUI);
        ztp_records.push(ZtpRecord {
            device_name: device.name.clone().to_owned(),
            config_file: format!("{}.conf", &device.name),
            ipv4_address: get_ipv4_addr(&mgmt_net.v4.prefix, device_ip_idx)?,
            mac_address: mac_address.to_string(),
            ztp_method: device_model.ztp_method.clone(),
            ssh_port: 22,
        });

        let mut interfaces: Vec<Interface> = vec![];

        // Management Interfaces
        if device_model.dedicated_management_interface {
            interfaces.push(Interface {
                name: dasher(&device_model.management_interface.to_string()),
                num: 0,
                mtu: device_model.interface_mtu,
                mac_address: mac_address.to_string(),
                connection_type: ConnectionTypes::Management,
                interface_connection: None,
            });
        }

        // Reserved Interfaces
        if device_model.reserved_interface_count > 0 {
            for i in device_model.first_interface_index..device_model.reserved_interface_count {
                interfaces.push(Interface {
                    name: format!("int{i}"),
                    num: i,
                    mtu: device_model.interface_mtu,
                    mac_address: random_mac(KVM_OUI),
                    connection_type: ConnectionTypes::Reserved,
                    interface_connection: None,
                });
            }
        }

        let end_iface_index = if device_model.first_interface_index == 0 {
            device_model.interface_count - 1
        } else {
            device_model.interface_count
        };
        for i in device_model.first_interface_index..=end_iface_index {
            // When device does not have a dedicated management interface the first_interface_index
            // Is assigned as a management interface
            if !device_model.dedicated_management_interface
                && i == device_model.first_interface_index
            {
                interfaces.push(Interface {
                    name: dasher(&device_model.management_interface.to_string()),
                    num: device_model.first_interface_index,
                    mtu: device_model.interface_mtu,
                    mac_address: mac_address.to_string(),
                    connection_type: ConnectionTypes::Management,
                    interface_connection: None,
                });
                continue;
            }
            // Device to device links
            if !links_detailed.is_empty() {
                let mut p2p_connection = false;
                for l in links_detailed.iter() {
                    // Device is source in manifest
                    if l.dev_a == device.name && i == l.int_a_idx {
                        let source_id = dev_id_map.get(&l.dev_b).ok_or_else(|| {
                            anyhow::anyhow!("Connection dev_b not found: {}", l.dev_b)
                        })?;
                        let local_id = dev_id_map.get(&device.name).unwrap().to_owned(); // should never error
                        let interface_connection = InterfaceConnection {
                            local_id,
                            local_port: id_to_port(i),
                            local_loopback: get_ip(local_id).to_string(),
                            source_id: source_id.to_owned(),
                            source_port: id_to_port(l.int_b_idx),
                            source_loopback: get_ip(source_id.to_owned()).to_string(),
                        };
                        interfaces.push(Interface {
                            name: dasher(&l.int_a),
                            num: i,
                            mtu: device_model.interface_mtu,
                            mac_address: random_mac(KVM_OUI),
                            connection_type: ConnectionTypes::Peer,
                            interface_connection: Some(interface_connection),
                        });
                        p2p_connection = true;
                        break;
                    // Device is destination in manifest
                    } else if l.dev_b == device.name && i == l.int_b_idx {
                        let source_id = dev_id_map.get(&l.dev_a).ok_or_else(|| {
                            anyhow::anyhow!("Connection dev_a not found: {}", l.dev_a)
                        })?;
                        let local_id = dev_id_map.get(&device.name).unwrap().to_owned(); // should never error
                        let interface_connection = InterfaceConnection {
                            local_id,
                            local_port: id_to_port(i),
                            local_loopback: get_ip(local_id).to_string(),
                            source_id: source_id.to_owned(),
                            source_port: id_to_port(l.int_a_idx),
                            source_loopback: get_ip(source_id.to_owned()).to_string(),
                        };
                        interfaces.push(Interface {
                            name: dasher(&l.int_b),
                            num: i,
                            mtu: device_model.interface_mtu,
                            mac_address: random_mac(KVM_OUI),
                            connection_type: ConnectionTypes::Peer,
                            interface_connection: Some(interface_connection),
                        });
                        p2p_connection = true;
                        break;
                    }
                }
                if !p2p_connection {
                    // Interface not defined in manifest so disable.
                    interfaces.push(Interface {
                        name: dasher(&interface_from_idx(&device.model, i)?),
                        num: i,
                        mtu: device_model.interface_mtu,
                        mac_address: random_mac(KVM_OUI),
                        connection_type: ConnectionTypes::Disabled,
                        interface_connection: None,
                    })
                }
            } else {
                interfaces.push(Interface {
                    name: dasher(&interface_from_idx(&device.model, i)?),
                    num: i,
                    mtu: device_model.interface_mtu,
                    mac_address: random_mac(KVM_OUI),
                    connection_type: ConnectionTypes::Disabled,
                    interface_connection: None,
                })
            }
        }

        // Container based image run in FlatCar linux.
        // Set the source disk to the latests FlatCar image.
        let src_boot_disk = if device_model.kind == DeviceKind::Container {
            format!(
                "{}/{}/{}/virtioa.qcow2",
                sherpa.images_dir,
                DeviceModels::FlatcarLinux,
                "latest"
            )
        } else {
            format!(
                "{}/{}/{}/virtioa.qcow2",
                sherpa.images_dir, device_model.name, device_model.version
            )
        };
        let dst_boot_disk = format!("{SHERPA_STORAGE_POOL_PATH}/{vm_name}-hdd.qcow2");
        copy_disks.push(CloneDisk {
            src: src_boot_disk,
            dst: dst_boot_disk.clone(),
        });

        // CDROM ISO
        let (mut src_cdrom_iso, mut dst_cdrom_iso) = match &device_model.cdrom {
            Some(src_iso) => {
                let src = format!(
                    "{}/{}/{}/{}",
                    sherpa.images_dir, device_model.name, device_model.version, src_iso
                );
                let dst = format!("{SHERPA_STORAGE_POOL_PATH}/{vm_name}.iso");
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

        if device_model.ztp_enable {
            ztp_devices.push(device);
            let device_ipv4_address = ztp_records
                .iter()
                .find(|r| r.device_name == device.name)
                .map(|r| r.ipv4_address);
            match device_model.ztp_method {
                ZtpMethods::CloudInit => {
                    term_msg_underline("Creating Cloud-Init disks");
                    // generate the template
                    println!("Creating Cloud-Init config {}", device.name);
                    let dir = format!("{TEMP_DIR}/{vm_name}");
                    match device.model {
                        DeviceModels::CentosLinux
                        | DeviceModels::AlmaLinux
                        | DeviceModels::RockyLinux
                        | DeviceModels::FedoraLinux
                        | DeviceModels::OpensuseLinux
                        | DeviceModels::RedhatLinux
                        | DeviceModels::SuseLinux
                        | DeviceModels::UbuntuLinux
                        | DeviceModels::FreeBsd
                        | DeviceModels::OpenBsd
                        | DeviceModels::NetBsd
                        | DeviceModels::DragonflyBsd
                        | DeviceModels::WindowsServer => {
                            let (admin_group, shell) = match device_model.os_variant {
                                OsVariants::Bsd => ("wheel".to_string(), "/bin/sh".to_string()),
                                _ => ("sudo".to_string(), "/bin/bash".to_string()),
                            };
                            let mut cloud_init_user = CloudInitUser::sherpa()?;
                            cloud_init_user.groups = vec![admin_group];
                            cloud_init_user.shell = shell;

                            let cloud_init_config = CloudInitConfig {
                                hostname: device.name.clone(),
                                fqdn: format!("{}.{}", device.name.clone(), SHERPA_DOMAIN_NAME),
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

                            if device_ipv4_address.is_some() {
                                let ztp_interface = CloudInitNetwork::ztp_interface(
                                    // This should always be Some
                                    device_ipv4_address.unwrap(),
                                    mac_address,
                                    mgmt_net.v4.clone(),
                                );
                                let cloud_network_config = ztp_interface.to_string()?;
                                create_file(&network_config, cloud_network_config)?;
                            }

                            create_ztp_iso(&format!("{}/{}", dir, ZTP_ISO), dir)?
                        }

                        DeviceModels::AlpineLinuxv => {
                            let meta_data = MetaDataConfig {
                                instance_id: format!("iid-{}", device.name.clone(),),
                                local_hostname: format!(
                                    "{}.{}",
                                    device.name.clone(),
                                    SHERPA_DOMAIN_NAME
                                ),
                            };
                            let mut cloud_init_user = CloudInitUser::sherpa()?;
                            cloud_init_user.shell = "/bin/sh".to_string();
                            cloud_init_user.groups = vec!["wheel".to_string()];
                            let cloud_init_config = CloudInitConfig {
                                hostname: device.name.clone(),
                                fqdn: format!("{}.{}", device.name.clone(), SHERPA_DOMAIN_NAME),
                                manage_etc_hosts: true,
                                ssh_pwauth: true,
                                users: vec![cloud_init_user],
                                ..Default::default()
                            };
                            let meta_data_config = meta_data.to_string()?;
                            let user_data_config = cloud_init_config.to_string()?;

                            let user_data = format!("{dir}/{CLOUD_INIT_USER_DATA}");
                            let meta_data = format!("{dir}/{CLOUD_INIT_META_DATA}");
                            create_dir(&dir)?;
                            create_file(&user_data, user_data_config)?;
                            // create_file(&user_data, rendered_template)?;
                            create_file(&meta_data, meta_data_config)?;
                            create_ztp_iso(&format!("{}/{}", dir, ZTP_ISO), dir)?
                        }
                        _ => {
                            anyhow::bail!(
                                "CDROM ZTP method not supported for {}",
                                device_model.name
                            );
                        }
                    }
                    src_cdrom_iso = Some(format!("{TEMP_DIR}/{vm_name}/{ZTP_ISO}"));
                    dst_cdrom_iso = Some(format!("{SHERPA_STORAGE_POOL_PATH}/{vm_name}.iso"));
                }
                ZtpMethods::Cdrom => {
                    term_msg_underline("Creating ZTP disks");
                    // generate the template
                    println!("Creating ZTP config {}", device.name);
                    let mut user = sherpa_user.clone();
                    let dir = format!("{TEMP_DIR}/{vm_name}");

                    match device.model {
                        DeviceModels::CiscoCsr1000v
                        | DeviceModels::CiscoCat8000v
                        | DeviceModels::CiscoCat9000v => {
                            let license_boot_command =
                                if device.model == DeviceModels::CiscoCat8000v {
                                    Some(
                                        "license boot level network-premier addon dna-premier"
                                            .to_string(),
                                    )
                                } else if device.model == DeviceModels::CiscoCat9000v {
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
                                hostname: device.name.clone(),
                                user,
                                mgmt_interface: device_model.management_interface.to_string(),
                                dns: dns.clone(),
                                license_boot_command,
                                mgmt_ipv4_address: device_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let c = CISCO_IOSXE_ZTP_CONFIG.replace("-", "_");
                            let ztp_config = format!("{dir}/{c}");
                            create_dir(&dir)?;
                            create_file(&ztp_config, rendered_template)?;
                            create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?
                        }
                        DeviceModels::CiscoAsav => {
                            let key_hash = pub_ssh_key_to_sha256_hash(&user.ssh_public_key.key)?;
                            user.ssh_public_key.key = key_hash;
                            let t = CiscoAsavZtpTemplate {
                                hostname: device.name.clone(),
                                user,
                                dns: dns.clone(),
                                mgmt_ipv4_address: device_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{CISCO_ASAV_ZTP_CONFIG}");
                            create_dir(&dir)?;
                            create_file(&ztp_config, rendered_template)?;
                            create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?
                        }
                        DeviceModels::CiscoNexus9300v => {
                            let t = CiscoNxosZtpTemplate {
                                hostname: device.name.clone(),
                                user,
                                dns: dns.clone(),
                                mgmt_ipv4_address: device_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{CISCO_NXOS_ZTP_CONFIG}");
                            create_dir(&dir)?;
                            create_file(&ztp_config, rendered_template)?;
                            create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?
                        }
                        DeviceModels::CiscoIosxrv9000 => {
                            let t = CiscoIosxrZtpTemplate {
                                hostname: device.name.clone(),
                                user,
                                dns: dns.clone(),
                                mgmt_ipv4_address: device_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = t.render()?;
                            let ztp_config = format!("{dir}/{CISCO_IOSXR_ZTP_CONFIG}");
                            create_dir(&dir)?;
                            create_file(&ztp_config, rendered_template)?;
                            create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?
                        }
                        DeviceModels::JuniperVsrxv3
                        | DeviceModels::JuniperVrouter
                        | DeviceModels::JuniperVswitch => {
                            let t = JunipervJunosZtpTemplate {
                                hostname: device.name.clone(),
                                user,
                                mgmt_interface: device_model.management_interface.to_string(),
                                mgmt_ipv4_address: device_ipv4_address,
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
                                device_model.name
                            );
                        }
                    };
                    src_cdrom_iso = Some(format!("{TEMP_DIR}/{vm_name}/{ZTP_ISO}"));
                    dst_cdrom_iso = Some(format!("{SHERPA_STORAGE_POOL_PATH}/{vm_name}-cfg.iso"));
                }
                ZtpMethods::Tftp => {
                    // generate the template
                    println!("Creating ZTP config {}", device.name);
                    let user = sherpa_user.clone();
                    let dir = format!("{TEMP_DIR}/{ZTP_DIR}/{TFTP_DIR}");

                    match device.model {
                        DeviceModels::AristaVeos => {
                            let arista_template = AristaVeosZtpTemplate {
                                hostname: device.name.clone(),
                                user: user.clone(),
                                dns: dns.clone(),
                                mgmt_ipv4_address: device_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let rendered_template = arista_template.render()?;
                            let ztp_config = format!("{dir}/{}.conf", device.name);
                            create_dir(&dir)?;
                            create_file(&ztp_config, rendered_template)?;
                        }
                        DeviceModels::ArubaAoscx => {
                            let aruba_template = ArubaAoscxTemplate {
                                hostname: device.name.clone(),
                                user: user.clone(),
                                dns: dns.clone(),
                                mgmt_ipv4_address: device_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let aruba_rendered_template = aruba_template.render()?;
                            let ztp_config = format!("{dir}/{}.conf", device.name);
                            create_dir(&dir)?;
                            create_file(&ztp_config, aruba_rendered_template)?;
                        }
                        DeviceModels::JuniperVevolved => {
                            let juniper_template = JunipervJunosZtpTemplate {
                                hostname: device.name.clone(),
                                user: sherpa_user.clone(),
                                mgmt_interface: device_model.management_interface.to_string(),
                                mgmt_ipv4_address: device_ipv4_address,
                                mgmt_ipv4: mgmt_net.v4.clone(),
                            };
                            let juniper_rendered_template = juniper_template.render()?;
                            let ztp_config = format!("{dir}/{}.conf", device.name);
                            create_dir(&dir)?;
                            create_file(&ztp_config, juniper_rendered_template)?;
                        }
                        _ => {
                            anyhow::bail!(
                                "Tftp ZTP method not supported for {}",
                                device_model.name
                            );
                        }
                    }
                }
                ZtpMethods::Http => {
                    // generate the template
                    println!("Creating ZTP config {}", device.name);
                    let _user = sherpa_user.clone();
                    let dir = format!("{TEMP_DIR}/{ZTP_DIR}/{DEVICE_CONFIGS_DIR}");

                    match device.model {
                        DeviceModels::SonicLinux => {
                            let sonic_ztp_file_map =
                                SonicLinuxZtp::file_map(&device.name, &mgmt_net.v4.boot_server);

                            let ztp_init = format!("{dir}/{}.conf", &device.name);
                            let sonic_ztp = SonicLinuxZtp {
                                hostname: device.name.clone(),
                                mgmt_ipv4: mgmt_net.v4.clone(),
                                mgmt_ipv4_address: device_ipv4_address,
                            };
                            let ztp_config = format!("{dir}/{}_config_db.json", &device.name);
                            create_dir(&dir)?;
                            create_file(&ztp_init, sonic_ztp_file_map)?;
                            create_file(&ztp_config, sonic_ztp.config())?;
                        }
                        _ => {
                            anyhow::bail!(
                                "HTTP ZTP method not supported for {}",
                                device_model.name
                            );
                        }
                    }
                }
                ZtpMethods::Disk => {
                    println!("Creating ZTP config {}", device.name);
                    let mut user = sherpa_user.clone();

                    let dir = format!("{TEMP_DIR}/{vm_name}");
                    match device.model {
                        DeviceModels::CiscoIosv => {
                            let key_hash = pub_ssh_key_to_md5_hash(&user.ssh_public_key.key)?;
                            user.ssh_public_key.key = key_hash;
                            let t = CiscoIosvZtpTemplate {
                                hostname: device.name.clone(),
                                user,
                                mgmt_interface: device_model.management_interface.to_string(),
                                dns: dns.clone(),
                                mgmt_ipv4_address: device_ipv4_address,
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
                            let dst_disk = format!("{dir}/{SHERPA_BLANK_DISK_DIR}-cfg.img");

                            // Create a copy of the disk base image
                            copy_file(&src_disk, &dst_disk)?;
                            // copy file to disk disk
                            copy_to_dos_image(&ztp_config, &dst_disk, "/")?;

                            src_config_disk = Some(dst_disk.to_owned());
                            dst_config_disk =
                                Some(format!("{SHERPA_STORAGE_POOL_PATH}/{vm_name}-cfg.img"));
                        }
                        DeviceModels::CiscoIosvl2 => {
                            let key_hash = pub_ssh_key_to_md5_hash(&user.ssh_public_key.key)?;
                            user.ssh_public_key.key = key_hash;
                            let t = CiscoIosvl2ZtpTemplate {
                                hostname: device.name.clone(),
                                user,
                                mgmt_interface: device_model.management_interface.to_string(),
                                dns: dns.clone(),
                                mgmt_ipv4_address: device_ipv4_address,
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
                            let dst_disk = format!("{dir}/{SHERPA_BLANK_DISK_DIR}-cfg.img");

                            // Create a copy of the hdd base image
                            copy_file(&src_disk, &dst_disk)?;
                            // copy file to hdd disk
                            copy_to_dos_image(&ztp_config, &dst_disk, "/")?;

                            src_config_disk = Some(dst_disk.to_owned());
                            dst_config_disk =
                                Some(format!("{SHERPA_STORAGE_POOL_PATH}/{vm_name}-cfg.img"));
                        }
                        _ => {
                            anyhow::bail!(
                                "Disk ZTP method not supported for {}",
                                device_model.name
                            );
                        }
                    }
                }
                ZtpMethods::Usb => {
                    // generate the template
                    println!("Creating ZTP config {}", device.name);
                    let user = sherpa_user.clone();
                    let dir = format!("{TEMP_DIR}/{vm_name}");

                    match device_model.name {
                        DeviceModels::CumulusLinux => {
                            let t = CumulusLinuxZtpTemplate {
                                hostname: device.name.clone(),
                                user,
                                dns: dns.clone(),
                                mgmt_ipv4_address: device_ipv4_address,
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
                                Some(format!("{SHERPA_STORAGE_POOL_PATH}/{vm_name}-cfg.img"));
                        }
                        DeviceModels::JuniperVevolved => {
                            let t = JunipervJunosZtpTemplate {
                                hostname: device.name.clone(),
                                user,
                                mgmt_interface: device_model.management_interface.to_string(),
                                mgmt_ipv4_address: device_ipv4_address,
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
                                Some(format!("{SHERPA_STORAGE_POOL_PATH}/{vm_name}-cfg.img"));
                        }
                        _ => {
                            anyhow::bail!("USB ZTP method not supported for {}", device_model.name);
                        }
                    }
                }
                ZtpMethods::Ignition => {
                    term_msg_underline("Creating ZTP disks");
                    // generate the template
                    println!("Creating ZTP config {}", device.name);
                    let user = sherpa_user.clone();
                    let dir = format!("{TEMP_DIR}/{vm_name}");
                    let dev_name = device.name.clone();
                    // Add the ignition config

                    let mut authorized_keys = vec![format!(
                        "{} {} {}",
                        user.ssh_public_key.algorithm,
                        user.ssh_public_key.key,
                        user.ssh_public_key.comment.unwrap_or("".to_owned())
                    )];

                    let manifest_authorized_keys: Vec<String> =
                        device.ssh_authorized_keys.clone().unwrap_or(vec![]);

                    let manifest_authorized_key_files: Vec<String> = device
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
                    let manifest_text_files: Vec<IgnitionFile> = device
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

                    let manifest_binary_disk_files = device.binary_files.clone().unwrap_or(vec![]);

                    let manifest_systemd_units: Vec<IgnitionUnit> = device
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

                    match device.model {
                        DeviceModels::NokiaSrlinux => {
                            let srlinux_unit = IgnitionUnit::srlinux();
                            let container_disk_unit = IgnitionUnit::mount_container_disk();

                            let container_disk = IgnitionFileSystem::default();
                            let ignition_config = IgnitionConfig::new(
                                vec![ignition_user],
                                vec![sudo_config_file, hostname_file],
                                vec![],
                                vec![container_disk_unit, srlinux_unit],
                                vec![],
                                vec![container_disk],
                            );
                            let flatcar_config = ignition_config.to_json_pretty()?;
                            let src_ztp_file = format!("{dir}/{ZTP_JSON}");
                            let dst_ztp_file =
                                format!("{SHERPA_STORAGE_POOL_PATH}/{vm_name}-cfg.ign");

                            create_dir(&dir)?;
                            create_file(&src_ztp_file, flatcar_config)?;

                            let src_container_disk = format!(
                                "{}/{}/{}/{}",
                                &sherpa.images_dir,
                                device_model.name,
                                device_model.version,
                                CONTAINER_DISK_NAME,
                            );

                            src_config_disk = Some(src_container_disk.to_owned());
                            dst_config_disk = Some(format!(
                                "{SHERPA_STORAGE_POOL_PATH}/{vm_name}-{CONTAINER_DISK_NAME}"
                            ));

                            src_ignition_disk = Some(src_ztp_file.to_owned());
                            dst_ignition_disk = Some(dst_ztp_file.to_owned());
                        }

                        DeviceModels::AristaCeos => {
                            let ceos_unit = IgnitionUnit::ceos();
                            let container_disk_unit = IgnitionUnit::mount_container_disk();

                            let container_disk = IgnitionFileSystem::default();
                            let ignition_config = IgnitionConfig::new(
                                vec![ignition_user],
                                vec![sudo_config_file, hostname_file],
                                vec![],
                                vec![container_disk_unit, ceos_unit],
                                vec![],
                                vec![container_disk],
                            );
                            let flatcar_config = ignition_config.to_json_pretty()?;
                            let src_ztp_file = format!("{dir}/{ZTP_JSON}");
                            let dst_ztp_file =
                                format!("{SHERPA_STORAGE_POOL_PATH}/{vm_name}-cfg.ign");

                            create_dir(&dir)?;
                            create_file(&src_ztp_file, flatcar_config)?;

                            let src_container_disk = format!(
                                "{}/{}/{}/{}",
                                &sherpa.images_dir,
                                device_model.name,
                                device_model.version,
                                CONTAINER_DISK_NAME,
                            );

                            src_config_disk = Some(src_container_disk.to_owned());
                            dst_config_disk = Some(format!(
                                "{SHERPA_STORAGE_POOL_PATH}/{vm_name}-{CONTAINER_DISK_NAME}"
                            ));

                            src_ignition_disk = Some(src_ztp_file.to_owned());
                            dst_ignition_disk = Some(dst_ztp_file.to_owned());
                        }
                        DeviceModels::FlatcarLinux => {
                            let mut units = vec![];
                            units.push(IgnitionUnit::mount_container_disk());
                            units.extend(manifest_systemd_units);

                            let container_disk = IgnitionFileSystem::default();

                            let mut files = vec![sudo_config_file, hostname_file, disable_update];
                            files.extend(manifest_text_files);

                            if device_ipv4_address.is_some() {
                                files.push(IgnitionFile::ztp_interface(
                                    // This should always be Some
                                    device_ipv4_address.unwrap(),
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
                                format!("{SHERPA_STORAGE_POOL_PATH}/{vm_name}-cfg.ign");

                            create_dir(&dir)?;
                            create_file(&src_ztp_file, flatcar_config)?;

                            // Copy a blank disk to to .tmp directory
                            let src_data_disk = format!(
                                "{}/{}/{}",
                                &sherpa.images_dir,
                                SHERPA_BLANK_DISK_DIR,
                                SHERPA_BLANK_DISK_EXT4_500MB
                            );
                            let dst_disk = format!("{dir}/{vm_name}-{CONTAINER_DISK_NAME}");

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
                                "{SHERPA_STORAGE_POOL_PATH}/{vm_name}-{CONTAINER_DISK_NAME}"
                            ));

                            src_ignition_disk = Some(src_ztp_file.to_owned());
                            dst_ignition_disk = Some(dst_ztp_file.to_owned());
                        }
                        _ => {
                            anyhow::bail!(
                                "Ignition ZTP method not supported for {}",
                                device_model.name
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        // ISO
        if dst_cdrom_iso.is_some() {
            copy_disks.push(CloneDisk {
                // These should always have a value.
                src: src_cdrom_iso.unwrap(),
                dst: dst_cdrom_iso.clone().unwrap(),
            });
            disks.push(DeviceDisk {
                disk_device: DiskDevices::Cdrom,
                driver_name: DiskDrivers::Qemu,
                driver_format: DiskFormats::Raw,
                // These should always have a value.
                src_file: dst_cdrom_iso.clone().unwrap(),
                target_dev: DiskTargets::target(&cdrom_bus, disks.len() as u8)?,
                target_bus: cdrom_bus.clone(),
            });
        }

        // Hdd
        disks.push(DeviceDisk {
            disk_device: DiskDevices::File,
            driver_name: DiskDrivers::Qemu,
            driver_format: DiskFormats::Qcow2,
            src_file: dst_boot_disk.clone(),
            target_dev: DiskTargets::target(&hdd_bus, disks.len() as u8)?,
            target_bus: hdd_bus.clone(),
        });

        // Data Disk
        if let Some(dst_config_disk) = dst_config_disk {
            copy_disks.push(CloneDisk {
                // These should always have a value.
                src: src_config_disk.unwrap(),
                dst: dst_config_disk.clone(),
            });
            disks.push(DeviceDisk {
                disk_device: DiskDevices::File,
                driver_name: DiskDrivers::Qemu,
                driver_format: DiskFormats::Raw,
                // These should always have a value.
                src_file: dst_config_disk.clone(),
                target_dev: DiskTargets::target(&hdd_bus, disks.len() as u8)?,
                target_bus: hdd_bus.clone(),
            });
        }

        // USB
        if let Some(dst_usb_disk) = dst_usb_disk {
            copy_disks.push(CloneDisk {
                // These should always have a value.
                src: src_usb_disk.unwrap(),
                dst: dst_usb_disk.clone(),
            });
            disks.push(DeviceDisk {
                disk_device: DiskDevices::File,
                driver_name: DiskDrivers::Qemu,
                driver_format: DiskFormats::Raw,
                // These should always have a value.
                src_file: dst_usb_disk.clone(),
                target_dev: DiskTargets::target(&DiskBuses::Usb, disks.len() as u8)?,
                target_bus: DiskBuses::Usb,
            });
        }

        // Ignition
        if dst_ignition_disk.is_some() {
            copy_disks.push(CloneDisk {
                // These should always have a value.
                src: src_ignition_disk.unwrap(),
                dst: dst_ignition_disk.clone().unwrap(),
            });
            disks.push(DeviceDisk {
                disk_device: DiskDevices::File,
                driver_name: DiskDrivers::Qemu,
                driver_format: DiskFormats::Raw,
                // These should always have a value.
                src_file: dst_ignition_disk.clone().unwrap(),
                target_dev: DiskTargets::target(&DiskBuses::Sata, disks.len() as u8)?,
                target_bus: DiskBuses::Sata,
            });
        }

        let qemu_commands = match device_model.name {
            DeviceModels::JuniperVrouter => QemuCommand::juniper_vrouter(),
            DeviceModels::JuniperVswitch => QemuCommand::juniper_vswitch(),
            DeviceModels::JuniperVevolved => QemuCommand::juniper_vevolved(),
            DeviceModels::FlatcarLinux | DeviceModels::NokiaSrlinux | DeviceModels::AristaCeos => {
                QemuCommand::ignition_config(&dst_ignition_disk.clone().unwrap())
            }
            _ => {
                vec![]
            }
        };

        let device_id = dev_id_map.get(&device.name).unwrap().to_owned(); // should never error
        let domain = DomainTemplate {
            qemu_bin: config.qemu_bin.clone(),
            name: vm_name,
            memory: device.memory.unwrap_or(device_model.memory),
            cpu_architecture: device_model.cpu_architecture.clone(),
            cpu_model: device_model.cpu_model.clone(),
            machine_type: device_model.machine_type.clone(),
            cpu_count: device.cpu_count.unwrap_or(device_model.cpu_count),
            vmx_enabled: device_model.vmx_enabled,
            bios: device_model.bios.clone(),
            disks,
            interfaces,
            interface_type: device_model.interface_type.clone(),
            loopback_ipv4: get_ip(device_id).to_string(),
            telnet_port: TELNET_PORT,
            qemu_commands,
            lab_id: lab_id.to_string(),
        };

        domains.push(domain);
    }

    create_ztp_files(&mgmt_net, &sherpa_user, &dns, &ztp_records)?;
    create_boot_containers(&docker_conn, &mgmt_net, lab_id).await?;

    // Clone disks in parallel
    term_msg_underline("Cloning Disks");
    let disk_handles: Vec<_> = copy_disks
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
    term_msg_underline("Creating VMs");
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
            create_file(".tmp/testbed.yaml", pyats_yaml)?;
        }

        term_msg_underline("Creating SSH Config File");
        let ssh_config_template = SshConfigTemplate {
            ztp_records: ztp_records.clone(),
        };
        let rendered_template = ssh_config_template.render()?;
        create_file(
            &format!("{TEMP_DIR}/{SHERPA_SSH_CONFIG_FILE}"),
            rendered_template,
        )?;
    }

    // Check if VMs are ready
    term_msg_underline("Checking VM Readiness");
    let start_time = Instant::now();
    let timeout = Duration::from_secs(READINESS_TIMEOUT); // 10 minutes
    let mut connected_devices = std::collections::HashSet::new();
    let mut device_ip_map = vec![];

    println!(
        "Waiting for VMs: {}",
        &ztp_devices
            .iter()
            .map(|x| x.name.as_str())
            .collect::<Vec<&str>>()
            .join(" ")
    );

    while start_time.elapsed() < timeout && connected_devices.len() < ztp_devices.len() {
        for device in &ztp_devices {
            if connected_devices.contains(&device.name) {
                continue;
            }

            let device_model = config
                .device_models
                .iter()
                .find(|d| d.name == device.model)
                .ok_or_else(|| anyhow::anyhow!("Device model not found: {}", device.model))?;
            let ssh_port = match device_model.name {
                DeviceModels::NokiaSrlinux => SSH_PORT_ALT,
                _ => SSH_PORT,
            };
            if let Some(vm_data) = ztp_records.iter().find(|x| x.device_name == device.name) {
                match tcp_connect(&vm_data.ipv4_address.to_string(), ssh_port)? {
                    true => {
                        println!("{} - Ready", &device.name);
                        connected_devices.insert(device.name.clone());
                        device_ip_map.push(DeviceConnection {
                            name: device.name.clone(),
                            ip_address: vm_data.ipv4_address.to_string(),
                            ssh_port,
                        });
                    }
                    false => {
                        println!("{} - Waiting for SSH", device.name);
                    }
                }
                // let leases = get_dhcp_leases(&config).await?;
                // if let Some(lease) = leases
                //     .iter()
                //     .find(|d| clean_mac(&d.mac_address) == clean_mac(&vm_data.mac_address))
                // {
                //     match tcp_connect(&lease.ipv4_address, ssh_port)? {
                //         true => {
                //             println!("{} - Ready", &device.name);
                //             connected_devices.insert(device.name.clone());
                //             device_ip_map.push(DeviceConnection {
                //                 name: device.name.clone(),
                //                 ip_address: lease.ipv4_address.clone(),
                //                 ssh_port,
                //             });
                //         }
                //         false => {
                //             println!("{} - Waiting for SSH", device.name);
                //         }
                //     }
                // } else {
                //     println!("{} - Still booting.", device.name);
                // }
            }
        }

        if connected_devices.len() < ztp_devices.len() {
            sleep(Duration::from_secs(READINESS_SLEEP));
        }
    }

    if connected_devices.len() == ztp_devices.len() {
        println!("All devices are ready!");
    } else {
        println!("Timeout reached. Not all devices are ready.");
        for device in &ztp_devices {
            if !connected_devices.contains(&device.name) {
                println!("Device is not ready: {}", device.name);
            }
        }
    }

    Ok(())
}
