use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use askama::Template;

use clap::{Parser, Subcommand};
use virt::storage_pool::StoragePool;
use virt::storage_vol::StorageVol;
use virt::sys;

use crate::core::konst::{
    ARISTA_OUI, ARISTA_VEOS_ZTP, ARISTA_VEOS_ZTP_SCRIPT, ARISTA_ZTP_DIR, ARUBA_OUI,
    ARUBA_ZTP_CONFIG, ARUBA_ZTP_DIR, BOOT_SERVER_MAC, BOOT_SERVER_NAME, CISCO_ASAV_ZTP_CONFIG,
    CISCO_IOSV_OUI, CISCO_IOSV_ZTP_CONFIG, CISCO_IOSXE_OUI, CISCO_IOSXE_ZTP_CONFIG,
    CISCO_IOSXR_OUI, CISCO_IOSXR_ZTP_CONFIG, CISCO_NXOS_OUI, CISCO_NXOS_ZTP_CONFIG, CISCO_ZTP_DIR,
    CLOUD_INIT_META_DATA, CLOUD_INIT_USER_DATA, CUMULUS_OUI, CUMULUS_ZTP, CUMULUS_ZTP_CONFIG,
    CUMULUS_ZTP_DIR, HTTP_PORT, JUNIPER_OUI, JUNIPER_ZTP_CONFIG, JUNIPER_ZTP_DIR, KVM_OUI,
    MTU_JUMBO_INT, READINESS_SLEEP, READINESS_TIMEOUT, SHERPA_BOXES_DIR, SHERPA_CONFIG_DIR,
    SHERPA_CONFIG_FILE, SHERPA_ISOLATED_NETWORK_BRIDGE, SHERPA_ISOLATED_NETWORK_NAME,
    SHERPA_MANAGEMENT_NETWORK_BRIDGE, SHERPA_MANAGEMENT_NETWORK_NAME, SHERPA_MANIFEST_FILE,
    SHERPA_SSH_CONFIG_FILE, SHERPA_SSH_PUBLIC_KEY_FILE, SHERPA_STORAGE_POOL,
    SHERPA_STORAGE_POOL_PATH, SHERPA_USB_DIR, SHERPA_USB_DISK, SHERPA_USERNAME, SSH_PORT,
    TELNET_PORT, TEMP_DIR, TFTP_PORT, ZTP_DIR, ZTP_ISO, ZTP_JSON,
};
use crate::core::{Config, Sherpa};
use crate::libvirt::{
    clone_disk, create_vm, delete_disk, get_mgmt_ip, ArubaAoscxTemplate, CiscoAsavZtpTemplate,
    CloudInitTemplate, DomainTemplate, IsolatedNetwork, JunipervJunosZtpTemplate,
    ManagementNetwork, Qemu, SherpaStoragePool,
};
use crate::model::{
    BiosTypes, ConnectionTypes, CpuArchitecture, DeviceModels, Interface, InterfaceTypes,
    MachineTypes, OsVariants, User, ZtpMethods,
};
use crate::topology::{ConnectionMap, Device, Manifest};
use crate::util::{
    base64_encode, copy_file, copy_to_usb_image, create_dir, create_file, create_ztp_iso,
    dir_exists, file_exists, fix_permissions_recursive, generate_ssh_keypair, get_id, get_ip,
    id_to_port, pub_ssh_key_to_md5_hash, pub_ssh_key_to_sha256_hash, random_mac, tcp_connect,
    term_msg_highlight, term_msg_surround, term_msg_underline, Contents as IgnitionFileContents,
    DeviceIp, File as IgnitionFile, IgnitionConfig, SshConfigTemplate, Unit as IgnitionUnit,
    User as IgnitionUser,
};

use crate::bootstrap::{
    arista_veos_ztp_script, AristaVeosZtpTemplate, CiscoIosXeZtpTemplate, CiscoIosvZtpTemplate,
    CiscoIosxrZtpTemplate, CiscoNxosZtpTemplate, CumulusLinuxZtpTemplate,
};

// Used to clone disk for VM creation
struct CloneDisk {
    src: String,
    dst: String,
}

struct _ZtpDisk {
    name: String,
    files: Vec<String>,
}

#[derive(Default, Debug, Parser)]
#[command(name = "sherpa")]
#[command(bin_name = "sherpa")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Sherpa - Network Lab Management", long_about = None)]
pub struct Cli {
    #[clap(subcommand)]
    commands: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Initialise a Sherpa environment
    Init {
        /// Name of the config file
        #[arg(default_value = SHERPA_CONFIG_FILE)]
        config_file: String,

        /// Name of the manifest file
        #[arg(default_value = SHERPA_MANIFEST_FILE)]
        manifest_file: String,

        /// Overwrite config file if one exists
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        force: bool,
    },
    /// Build environment
    Up {
        /// Name of the config file
        #[arg(default_value = SHERPA_CONFIG_FILE)]
        config_file: String,
    },
    /// Stop environment
    Down,
    /// Resume environment
    Resume,
    /// Destroy environment
    Destroy,
    /// Inspect environment
    Inspect,

    /// Fix up environment
    Doctor {
        /// Set base box permissions to read-only
        #[arg(long, action = clap::ArgAction::SetTrue)]
        boxes: bool,
    },

    /// Clean up environment
    Clean {
        /// Remove all devices, disks and networks
        #[arg(long, action = clap::ArgAction::SetTrue)]
        all: bool,
        /// Remove all disks
        #[arg(long, action = clap::ArgAction::SetTrue)]
        disks: bool,
        /// Remove all networks
        #[arg(long, action = clap::ArgAction::SetTrue)]
        networks: bool,
    },

    /// Import a disk image
    Import {
        /// Source path of the disk image
        #[arg(short, long)]
        src: String,
        /// Version of the device model
        #[arg(short, long)]
        version: String,
        /// Model of Device
        #[arg(short, long, value_enum)]
        model: DeviceModels,
        /// Import the disk image as the latest version
        #[arg(long, action = clap::ArgAction::SetTrue)]
        latest: bool,
    },

    /// Connect to a device via serial console over Telnet
    Console { name: String },

    /// SSH to a device.
    Ssh { name: String },
}
impl Default for Commands {
    fn default() -> Self {
        Commands::Init {
            config_file: SHERPA_CONFIG_FILE.to_owned(),
            manifest_file: SHERPA_MANIFEST_FILE.to_owned(),
            force: false,
        }
    }
}

impl Cli {
    pub fn run() -> Result<()> {
        let cli = Cli::parse();
        let qemu = Qemu::default();
        let mut sherpa = Sherpa::default();

        match &cli.commands {
            Commands::Init {
                config_file,
                manifest_file,
                force,
            } => {
                term_msg_surround("Sherpa Initializing");
                let qemu_conn = qemu.connect()?;

                sherpa.config_path = format!("{}/{}", sherpa.config_dir, config_file);

                term_msg_highlight("Creating Files");
                // Create the default config directories
                let config = if dir_exists(&sherpa.config_dir) && !*force {
                    println!("Directory path already exists: {}", sherpa.config_dir);
                    Config::load(&sherpa.config_path)?
                } else {
                    create_dir(&sherpa.config_dir)?;
                    create_dir(&sherpa.boxes_dir)?;
                    // box directories
                    let config = Config::default();
                    for device_model in &config.device_models {
                        create_dir(&format!(
                            "{}/{}/latest",
                            sherpa.boxes_dir, device_model.name
                        ))?;
                    }
                    config
                };

                // Initialize default files
                if file_exists(&sherpa.config_path) && !*force {
                    println!("Config file already exists: {}", sherpa.config_path);
                } else {
                    let config = Config {
                        name: config_file.to_owned(),
                        ..Default::default()
                    };
                    config.create(&sherpa.config_path)?;
                }

                if file_exists(manifest_file) && !*force {
                    println!("Manifest file already exists: {manifest_file}");
                } else {
                    let manifest = Manifest::default();
                    manifest.write_file()?;
                }

                // SSH Keys
                let ssh_pub_key_file =
                    format!("{}/{}", &sherpa.config_dir, SHERPA_SSH_PUBLIC_KEY_FILE);

                if file_exists(&ssh_pub_key_file) && !*force {
                    println!("SSH keys already exists: {ssh_pub_key_file}");
                } else {
                    term_msg_underline("Creating SSH Keypair");
                    generate_ssh_keypair(&sherpa.config_dir)?;
                }

                term_msg_highlight("Creating Networks");
                // Initialize the sherpa boot network
                if qemu_conn
                    .list_networks()?
                    .iter()
                    .any(|net| net == SHERPA_MANAGEMENT_NETWORK_NAME)
                {
                    println!("Network already exists: {SHERPA_MANAGEMENT_NETWORK_NAME}");
                } else {
                    println!("Creating network: {SHERPA_MANAGEMENT_NETWORK_NAME}");
                    let ipv4_network_size = config.management_prefix_ipv4.size();
                    let management_network = ManagementNetwork {
                        network_name: SHERPA_MANAGEMENT_NETWORK_NAME.to_owned(),
                        bridge_name: SHERPA_MANAGEMENT_NETWORK_BRIDGE.to_owned(),
                        ipv4_address: config.management_prefix_ipv4.nth(1).unwrap(),
                        ipv4_netmask: config.management_prefix_ipv4.mask(),
                        ipv4_default_gateway: config.management_prefix_ipv4.nth(1).unwrap(),
                        dhcp_start: config.management_prefix_ipv4.nth(5).unwrap(),
                        dhcp_end: config
                            .management_prefix_ipv4
                            .nth(ipv4_network_size - 2)
                            .unwrap(),
                        ztp_http_port: HTTP_PORT,
                        ztp_tftp_port: TFTP_PORT,
                        ztp_server_ipv4: config.management_prefix_ipv4.nth(5).unwrap(),
                    };
                    management_network.create(&qemu_conn)?;
                }

                // Create the isolated network
                if qemu_conn
                    .list_networks()?
                    .iter()
                    .any(|net| net == SHERPA_ISOLATED_NETWORK_NAME)
                {
                    println!("Network already exists: {SHERPA_ISOLATED_NETWORK_NAME}");
                } else {
                    println!("Creating network: {SHERPA_ISOLATED_NETWORK_NAME}");
                    let isolated_network = IsolatedNetwork {
                        network_name: SHERPA_ISOLATED_NETWORK_NAME.to_owned(),
                        bridge_name: SHERPA_ISOLATED_NETWORK_BRIDGE.to_owned(),
                    };
                    isolated_network.create(&qemu_conn)?;
                }
                let storage_pool = SherpaStoragePool {
                    name: SHERPA_STORAGE_POOL.to_owned(),
                    path: SHERPA_STORAGE_POOL_PATH.to_owned(),
                };
                storage_pool.create(&qemu_conn)?;
            }

            Commands::Up { config_file } => {
                term_msg_surround("Building environment");

                // TODO: allow config file to be specified.
                println!("Loading config");
                sherpa.config_path = format!("{}/{}", sherpa.config_dir, config_file);
                let config = Config::load(&sherpa.config_path)?;

                let qemu_conn = Arc::new(qemu.connect()?);

                println!("Loading manifest");
                let manifest = Manifest::load_file()?;

                let lab_id = get_id()?;

                let sherpa_user = User::default()?;

                // Create ZTP files
                term_msg_underline("Creating ZTP configs");

                // Aristra vEOS
                let arista_dir = format!("{TEMP_DIR}/{ZTP_DIR}/{ARISTA_ZTP_DIR}");
                create_dir(&arista_dir)?;

                // let arista_template = AristaVeosZtpTemplate {
                //     hostname: "veos-ztp".to_owned(),
                //     users: vec![sherpa_user.clone()],
                //     name_server: config.management_prefix_ipv4.nth(1).unwrap(),
                // };
                // let arista_rendered_template = arista_template.render()?;
                // let arista_ztp_config = format!("{arista_dir}/{ARISTA_VEOS_ZTP_CONFIG}");
                // create_file(&arista_ztp_config, arista_rendered_template.clone())?;

                let arista_ztp_file = format!("{arista_dir}/{ARISTA_VEOS_ZTP_SCRIPT}");
                let arista_ztp_script = arista_veos_ztp_script();
                create_file(&arista_ztp_file, arista_ztp_script.clone())?;

                // Aruba AOS
                let aruba_dir = format!("{TEMP_DIR}/{ZTP_DIR}/{ARUBA_ZTP_DIR}");
                create_dir(&aruba_dir)?;

                let aruba_template = ArubaAoscxTemplate {
                    hostname: "aos-ztp".to_owned(),
                    users: vec![sherpa_user.clone()],
                };
                let aruba_rendered_template = aruba_template.render()?;
                let aruba_ztp_config = format!("{aruba_dir}/{ARUBA_ZTP_CONFIG}");
                create_file(&aruba_ztp_config, aruba_rendered_template.clone())?;

                // Cumulus Linux
                let cumulus_dir = format!("{TEMP_DIR}/{ZTP_DIR}/{CUMULUS_ZTP_DIR}");
                create_dir(&cumulus_dir)?;

                let cumulus_template = CumulusLinuxZtpTemplate {
                    hostname: "cumulus-ztp".to_owned(),
                    users: vec![sherpa_user.clone()],
                    name_server: config.management_prefix_ipv4.nth(1).unwrap(),
                };
                let cumulus_rendered_template = cumulus_template.render()?;
                let cumulus_ztp_config = format!("{cumulus_dir}/{CUMULUS_ZTP_CONFIG}");
                create_file(&cumulus_ztp_config, cumulus_rendered_template.clone())?;

                // Cisco
                let cisco_dir = format!("{TEMP_DIR}/{ZTP_DIR}/{CISCO_ZTP_DIR}");
                create_dir(&cisco_dir)?;
                let mut cisco_user = sherpa_user.clone();
                cisco_user.ssh_public_key.key =
                    pub_ssh_key_to_md5_hash(&cisco_user.ssh_public_key.key)?;

                // IOSXE
                let cisco_iosxe_template = CiscoIosXeZtpTemplate {
                    hostname: "iosxe-ztp".to_owned(),
                    users: vec![cisco_user.clone()],
                    mgmt_interface: "GigabitEthernet1".to_owned(),
                    name_server: config.management_prefix_ipv4.nth(1).unwrap(),
                };
                let iosxe_rendered_template = cisco_iosxe_template.render()?;
                let cisco_iosxe_ztp_config = format!("{cisco_dir}/{CISCO_IOSXE_ZTP_CONFIG}");
                create_file(&cisco_iosxe_ztp_config, iosxe_rendered_template.clone())?;

                // IOSv
                let cisco_iosv_template = CiscoIosvZtpTemplate {
                    hostname: "iosv-ztp".to_owned(),
                    users: vec![cisco_user.clone()],
                    mgmt_interface: "GigabitEthernet0/0".to_owned(),
                    name_server: config.management_prefix_ipv4.nth(1).unwrap(),
                };
                let iosv_rendered_template = cisco_iosv_template.render()?;
                let cisco_iosv_ztp_config = format!("{cisco_dir}/{CISCO_IOSV_ZTP_CONFIG}");
                create_file(&cisco_iosv_ztp_config, iosv_rendered_template.clone())?;

                // Juniper vrouter
                let juniper_dir = format!("{TEMP_DIR}/{ZTP_DIR}/{JUNIPER_ZTP_DIR}");
                create_dir(&juniper_dir)?;

                let juniper_vjunos_template = JunipervJunosZtpTemplate {
                    hostname: "vjunos-ztp".to_owned(),
                    users: vec![sherpa_user.clone()],
                };
                let juniper_vjunos_rendered_template = juniper_vjunos_template.render()?;
                let juniper_vjunos_ztp_config = format!("{juniper_dir}/{JUNIPER_ZTP_CONFIG}");
                create_file(
                    &juniper_vjunos_ztp_config,
                    juniper_vjunos_rendered_template.clone(),
                )?;

                // Create a mapping of device name to device id.
                let dev_id_map: HashMap<String, u8> = manifest
                    .devices
                    .iter()
                    .map(|d| (d.name.clone(), d.id))
                    .collect();

                // let mut ztp_disks: Vec<ZtpDisk> = vec![];
                let mut copy_disks: Vec<CloneDisk> = vec![];
                let mut domains: Vec<DomainTemplate> = vec![];
                let user = User::default()?;
                for device in &manifest.devices {
                    let vm_name = format!("{}-{}", device.name, lab_id);

                    let device_model = config
                        .device_models
                        .iter()
                        .find(|d| d.name == device.device_model)
                        .ok_or_else(|| {
                            anyhow::anyhow!("Device model not found: {}", device.device_model)
                        })?;

                    let mac_address = match device.device_model {
                        DeviceModels::AristaVeos => random_mac(ARISTA_OUI),
                        DeviceModels::ArubaAoscx => random_mac(ARUBA_OUI),
                        DeviceModels::CiscoCat8000v
                        | DeviceModels::CiscoCat9000v
                        | DeviceModels::CiscoCsr1000v => random_mac(CISCO_IOSXE_OUI),
                        DeviceModels::CiscoIosv | DeviceModels::CiscoIosvl2 => {
                            random_mac(CISCO_IOSV_OUI)
                        }
                        DeviceModels::CiscoNexus9300v => random_mac(CISCO_NXOS_OUI),
                        DeviceModels::CiscoIosxrv9000 => random_mac(CISCO_IOSXR_OUI),
                        DeviceModels::JuniperVrouter
                        | DeviceModels::JuniperVswitch
                        | DeviceModels::JuniperVsrxv3 => random_mac(JUNIPER_OUI),
                        DeviceModels::CumulusLinux => random_mac(CUMULUS_OUI),
                        DeviceModels::FlatcarLinux => BOOT_SERVER_MAC.to_owned(),
                        _ => random_mac(KVM_OUI),
                    };

                    let mut interfaces: Vec<Interface> = vec![];

                    if device_model.management_interface {
                        interfaces.push(Interface {
                            name: "mgmt".to_owned(),
                            num: 0,
                            mtu: device_model.interface_mtu,
                            mac_address: mac_address.to_string(),
                            connection_type: ConnectionTypes::Management,
                            connection_map: None,
                        });
                    }
                    if device_model.reserved_interface_count > 0 {
                        for i in 0..device_model.reserved_interface_count {
                            interfaces.push(Interface {
                                name: "reserved".to_owned(),
                                num: i,
                                mtu: device_model.interface_mtu,
                                mac_address: random_mac(KVM_OUI),
                                connection_type: ConnectionTypes::Reserved,
                                connection_map: None,
                            });
                        }
                    }
                    for i in 0..device_model.interface_count {
                        match &manifest.connections {
                            Some(connections) => {
                                for c in connections {
                                    // Device is source in manifest
                                    if c.device_a == device.name && i == c.interface_a {
                                        let source_id =
                                            dev_id_map.get(&c.device_b).ok_or_else(|| {
                                                anyhow::anyhow!(
                                                    "Connection device_b not found: {}",
                                                    c.device_b
                                                )
                                            })?;
                                        let connection_map = ConnectionMap {
                                            local_id: device.id,
                                            local_port: id_to_port(i),
                                            local_loopback: get_ip(device.id).to_string(),
                                            source_id: source_id.to_owned(),
                                            source_port: id_to_port(c.interface_b),
                                            source_loopback: get_ip(source_id.to_owned())
                                                .to_string(),
                                        };
                                        interfaces.push(Interface {
                                            name: format!("{}{}", device_model.interface_prefix, i),
                                            num: i,
                                            mtu: device_model.interface_mtu,
                                            mac_address: random_mac(KVM_OUI),
                                            connection_type: ConnectionTypes::Peer,
                                            connection_map: Some(connection_map),
                                        })
                                    // Device is destination in manifest
                                    } else if c.device_b == device.name && i == c.interface_b {
                                        let source_id =
                                            dev_id_map.get(&c.device_a).ok_or_else(|| {
                                                anyhow::anyhow!(
                                                    "Connection device_a not found: {}",
                                                    c.device_a
                                                )
                                            })?;
                                        let connection_map = ConnectionMap {
                                            local_id: device.id,
                                            local_port: id_to_port(i),
                                            local_loopback: get_ip(device.id).to_string(),
                                            source_id: source_id.to_owned(),
                                            source_port: id_to_port(c.interface_a),
                                            source_loopback: get_ip(source_id.to_owned())
                                                .to_string(),
                                        };
                                        interfaces.push(Interface {
                                            name: format!("{}{}", device_model.interface_prefix, i),
                                            num: i,
                                            mtu: device_model.interface_mtu,
                                            mac_address: random_mac(KVM_OUI),
                                            connection_type: ConnectionTypes::Peer,
                                            connection_map: Some(connection_map),
                                        })
                                    } else {
                                        // Interface not defined in manifest so disable.
                                        interfaces.push(Interface {
                                            name: format!("{}{}", device_model.interface_prefix, i),
                                            num: i,
                                            mtu: device_model.interface_mtu,
                                            mac_address: random_mac(KVM_OUI),
                                            connection_type: ConnectionTypes::Disabled,
                                            connection_map: None,
                                        })
                                    }
                                }
                            }
                            None => interfaces.push(Interface {
                                name: format!("{}{}", device_model.interface_prefix, i),
                                num: i,
                                mtu: device_model.interface_mtu,
                                mac_address: random_mac(KVM_OUI),
                                connection_type: ConnectionTypes::Disabled,
                                connection_map: None,
                            }),
                        }
                    }

                    let src_boot_disk = format!(
                        "{}/{}/{}/virtioa.qcow2",
                        sherpa.boxes_dir, device_model.name, device_model.version
                    );
                    let dst_boot_disk = format!("{SHERPA_STORAGE_POOL_PATH}/{vm_name}.qcow2");
                    copy_disks.push(CloneDisk {
                        src: src_boot_disk,
                        dst: dst_boot_disk.clone(),
                    });

                    // CDROM ISO
                    let (mut src_cdrom_iso, mut dst_cdrom_iso) = match &device_model.cdrom {
                        Some(src_iso) => {
                            let src = format!(
                                "{}/{}/{}/{}",
                                sherpa.boxes_dir, device_model.name, device_model.version, src_iso
                            );
                            let dst = format!("{SHERPA_STORAGE_POOL_PATH}/{vm_name}.iso");
                            (Some(src), Some(dst))
                        }
                        None => (None, None),
                    };

                    let (mut src_usb_disk, mut dst_usb_disk) = (None::<String>, None::<String>);

                    if device_model.ztp_enable {
                        match device_model.ztp_method {
                            ZtpMethods::Cdrom => {
                                term_msg_underline("Creating ZTP disks");
                                // generate the template
                                println!("Creating ZTP config {}", device.name);
                                let mut user = user.clone();
                                let dir = format!("{TEMP_DIR}/{vm_name}");

                                match device.device_model {
                                    DeviceModels::CiscoCsr1000v
                                    | DeviceModels::CiscoCat8000v
                                    | DeviceModels::CiscoCat9000v => {
                                        let key_hash =
                                            pub_ssh_key_to_md5_hash(&user.ssh_public_key.key)?;
                                        user.ssh_public_key.key = key_hash;
                                        let t = CiscoIosXeZtpTemplate {
                                            hostname: device.name.clone(),
                                            users: vec![user],
                                            mgmt_interface: "GigabitEthernet1".to_owned(),
                                            name_server: config
                                                .management_prefix_ipv4
                                                .nth(1)
                                                .unwrap(),
                                        };
                                        let rendered_template = t.render()?;
                                        let c = CISCO_IOSXE_ZTP_CONFIG.replace("-", "_");
                                        let ztp_config = format!("{dir}/{c}");
                                        create_dir(&dir)?;
                                        create_file(&ztp_config, rendered_template)?;
                                        create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?
                                    }
                                    DeviceModels::CiscoAsav => {
                                        let key_hash =
                                            pub_ssh_key_to_sha256_hash(&user.ssh_public_key.key)?;
                                        user.ssh_public_key.key = key_hash;
                                        let t = CiscoAsavZtpTemplate {
                                            hostname: device.name.clone(),
                                            users: vec![user],
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
                                            users: vec![user],
                                            name_server: config
                                                .management_prefix_ipv4
                                                .nth(1)
                                                .unwrap(),
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
                                            users: vec![user],
                                            name_server: config
                                                .management_prefix_ipv4
                                                .nth(1)
                                                .unwrap(),
                                        };
                                        let rendered_template = t.render()?;
                                        let ztp_config = format!("{dir}/{CISCO_IOSXR_ZTP_CONFIG}");
                                        create_dir(&dir)?;
                                        create_file(&ztp_config, rendered_template)?;
                                        create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?
                                    }
                                    DeviceModels::JuniperVsrxv3 => {
                                        let t = JunipervJunosZtpTemplate {
                                            hostname: device.name.clone(),
                                            users: vec![user],
                                        };
                                        let rendered_template = t.render()?;
                                        let ztp_config = format!("{dir}/{JUNIPER_ZTP_CONFIG}");
                                        create_dir(&dir)?;
                                        create_file(&ztp_config, rendered_template)?;
                                        create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?
                                    }
                                    DeviceModels::CentosLinux
                                    | DeviceModels::FedoraLinux
                                    | DeviceModels::OpensuseLinux
                                    | DeviceModels::RedhatLinux
                                    | DeviceModels::SuseLinux
                                    | DeviceModels::UbuntuLinux => {
                                        let t = CloudInitTemplate {
                                            hostname: device.name.clone(),
                                            users: vec![user],
                                            password_auth: device_model.ztp_password_auth,
                                        };
                                        let rendered_template = t.render()?;
                                        let user_data = format!("{dir}/{CLOUD_INIT_USER_DATA}");
                                        let meta_data = format!("{dir}/{CLOUD_INIT_META_DATA}");
                                        create_dir(&dir)?;
                                        create_file(&user_data, rendered_template)?;
                                        create_file(&meta_data, "".to_string())?;
                                        create_ztp_iso(&format!("{}/{}", dir, ZTP_ISO), dir)?
                                    }
                                    _ => {
                                        anyhow::bail!(
                                            "CDROM ZTP method not supported for {}",
                                            device_model.name
                                        );
                                    }
                                };
                                src_cdrom_iso = Some(format!("{TEMP_DIR}/{vm_name}/{ZTP_ISO}"));
                                dst_cdrom_iso =
                                    Some(format!("{SHERPA_STORAGE_POOL_PATH}/{vm_name}.iso"));
                            }
                            ZtpMethods::Http => {
                                // generate the template
                                println!("Creating ZTP config {}", device.name);
                                let _user = user.clone();
                                let _dir = format!("{TEMP_DIR}/{ZTP_DIR}/{ARISTA_ZTP_DIR}");

                                match device_model.os_variant {
                                    OsVariants::Aos => {}
                                    OsVariants::Eos => {}
                                    _ => {
                                        anyhow::bail!(
                                            "HTTP ZTP method not supported for {}",
                                            device_model.name
                                        );
                                    }
                                }
                            }
                            ZtpMethods::Usb => {
                                // generate the template
                                println!("Creating ZTP config {}", device.name);
                                let user = user.clone();
                                let dir = format!("{TEMP_DIR}/{vm_name}");

                                match device_model.os_variant {
                                    OsVariants::CumulusLinux => {
                                        let t = CumulusLinuxZtpTemplate {
                                            hostname: device.name.clone(),
                                            users: vec![user],
                                            name_server: config
                                                .management_prefix_ipv4
                                                .nth(1)
                                                .unwrap(),
                                        };
                                        let rendered_template = t.render()?;
                                        let ztp_config = format!("{dir}/{CUMULUS_ZTP}");
                                        create_dir(&dir)?;
                                        create_file(&ztp_config, rendered_template)?;
                                        // clone USB disk
                                        let src_usb = format!(
                                            "{}/{}/{}",
                                            &sherpa.boxes_dir, SHERPA_USB_DIR, SHERPA_USB_DISK
                                        );
                                        let dst_usb = format!("{dir}/{SHERPA_USB_DISK}");

                                        // Create a copy of the usb base image
                                        copy_file(&src_usb, &dst_usb)?;
                                        // copy file to USB disk
                                        copy_to_usb_image(&ztp_config, &dst_usb, "/")?;

                                        src_usb_disk = Some(dst_usb.to_owned());
                                        dst_usb_disk = Some(format!(
                                            "{SHERPA_STORAGE_POOL_PATH}/{vm_name}.img"
                                        ));
                                    }
                                    OsVariants::Eos => {
                                        let t = AristaVeosZtpTemplate {
                                            hostname: device.name.clone(),
                                            users: vec![user],
                                            name_server: config
                                                .management_prefix_ipv4
                                                .nth(1)
                                                .unwrap(),
                                        };
                                        let rendered_template = t.render()?;
                                        let ztp_config = format!("{dir}/{ARISTA_VEOS_ZTP}");
                                        create_dir(&dir)?;
                                        create_file(&ztp_config, rendered_template)?;
                                        // clone USB disk
                                        let src_usb = format!(
                                            "{}/{}/{}",
                                            &sherpa.boxes_dir, SHERPA_USB_DIR, SHERPA_USB_DISK
                                        );
                                        let dst_usb = format!("{dir}/{SHERPA_USB_DISK}");

                                        // Create a copy of the usb base image
                                        copy_file(&src_usb, &dst_usb)?;
                                        // copy file to USB disk
                                        copy_to_usb_image(&ztp_config, &dst_usb, "/")?;

                                        src_usb_disk = Some(dst_usb.to_owned());
                                        dst_usb_disk = Some(format!(
                                            "{SHERPA_STORAGE_POOL_PATH}/{vm_name}.img"
                                        ));
                                    }
                                    _ => {
                                        anyhow::bail!(
                                            "HTTP ZTP method not supported for {}",
                                            device_model.name
                                        );
                                    }
                                }
                            }
                            ZtpMethods::Ignition => {
                                term_msg_underline("Creating ZTP disks");
                                // generate the template
                                println!("Creating ZTP config {}", device.name);
                                let _user = user.clone();
                                let _dir = format!("{TEMP_DIR}/{vm_name}");
                                match device.device_model {
                                    DeviceModels::FlatcarLinux => {
                                        // let ignition_user = IgnitionUser {
                                        //     name: user.username,
                                        //     ssh_authorized_keys: vec![format!(
                                        //         "{} {}",
                                        //         user.ssh_public_key.algorithm,
                                        //         user.ssh_public_key.key
                                        //     )],
                                        // };
                                        // let hostname_file = IgnitionFile {
                                        //     // filesystem: "root".to_owned(),
                                        //     path: "/etc/hostname".to_owned(),
                                        //     mode: 644,
                                        //     contents: IgnitionFileContents::new(
                                        //         "data:,boot-server",
                                        //     ),
                                        // };
                                        // let arista_ztp_base64 =
                                        //     base64_encode(&arista_rendered_template);
                                        // let arista_ztp_file = IgnitionFile {
                                        //     // filesystem: "root".to_owned(),
                                        //     path: format!("/opt/ztp/{ARISTA_VEOS_ZTP_CONFIG}"),
                                        //     mode: 644,
                                        //     contents: IgnitionFileContents::new(&format!(
                                        //         "data:;base64,{arista_ztp_base64}"
                                        //     )),
                                        // };
                                        // let cumulus_ztp_base64 =
                                        //     base64_encode(&cumulus_rendered_template);
                                        // let cumulus_ztp_file = IgnitionFile {
                                        //     // filesystem: "root".to_owned(),
                                        //     path: format!("/opt/ztp/{CUMULUS_ZTP_CONFIG}"),
                                        //     mode: 644,
                                        //     contents: IgnitionFileContents::new(&format!(
                                        //         "data:;base64,{cumulus_ztp_base64}"
                                        //     )),
                                        // };
                                        // let iosxe_ztp_base64 =
                                        //     base64_encode(&iosxe_rendered_template);
                                        // let iosxe_ztp_file = IgnitionFile {
                                        //     // filesystem: "root".to_owned(),
                                        //     path: format!("/opt/ztp/{CISCO_IOSXE_ZTP_CONFIG}"),
                                        //     mode: 644,
                                        //     contents: IgnitionFileContents::new(&format!(
                                        //         "data:;base64,{iosxe_ztp_base64}"
                                        //     )),
                                        // };
                                        // let iosv_ztp_base64 =
                                        //     base64_encode(&iosv_rendered_template);
                                        // let iosv_ztp_file = IgnitionFile {
                                        //     // filesystem: "root".to_owned(),
                                        //     path: format!("/opt/ztp/{CISCO_IOSV_ZTP_CONFIG}"),
                                        //     mode: 644,
                                        //     contents: IgnitionFileContents::new(&format!(
                                        //         "data:;base64,{iosv_ztp_base64}"
                                        //     )),
                                        // };

                                        // let config = IgnitionConfig::new(
                                        //     vec![ignition_user],
                                        //     vec![
                                        //         hostname_file,
                                        //         arista_ztp_file,
                                        //         cumulus_ztp_file,
                                        //         iosxe_ztp_file,
                                        //         iosv_ztp_file,
                                        //     ],
                                        //     vec![],
                                        //     // vec![link_default],
                                        // );
                                        // let flatcar_config = config.to_json_pretty()?;
                                        // let src_ztp_file = format!("{dir}/{ZTP_JSON}");
                                        // let dst_ztp_file =
                                        //     format!("{SHERPA_STORAGE_POOL_PATH}/{vm_name}.ign");

                                        // create_dir(&dir)?;
                                        // create_file(&src_ztp_file, flatcar_config)?;
                                        // copy_disks.push(CloneDisk {
                                        //     src: src_ztp_file,
                                        //     dst: dst_ztp_file.clone(),
                                        // });
                                    }
                                    _ => {
                                        anyhow::bail!(
                                            "Ignition ZTP method not supported for {}",
                                            device_model.name
                                        );
                                    }
                                }
                            }
                            // ZtpMethods::Usb => {
                            //     println!("Creating ZTP config {}", device.name);
                            //     let user = user.clone();
                            //     let dir = format!("{TEMP_DIR}/{vm_name}");

                            //     match device_model.os_variant {
                            //         OsVariants::CumulusLinux => {
                            //             let t = CumulusLinuxZtpTemplate {
                            //                 hostname: device.name.clone(),
                            //                 users: vec![user],
                            //             };
                            //             let rendered_template = t.render()?;
                            //             let ztp_config = format!("{dir}/{CUMULUS_ZTP_CONFIG}");
                            //             let ztp_iso = format!("{dir}/{ZTP_ISO}");
                            //             let src_ztp_usb = format!("{dir}/{ZTP_USB}");
                            //             create_dir(&dir)?;
                            //             create_file(&ztp_config, rendered_template)?;
                            //             create_ztp_iso(&ztp_iso, dir)?;
                            //             convert_iso_qcow2(&ztp_iso, &src_ztp_usb)?;

                            //             src_usb_disk = Some(src_ztp_usb);
                            //             dst_usb_disk =
                            //                 Some(format!("{SHERPA_STORAGE_POOL_PATH}/{vm_name}-ztp.qcow2"))
                            //         }
                            //         _ => {
                            //             anyhow::bail!(
                            //                 "USB ZTP method not supported for {}",
                            //                 device_model.name
                            //             );
                            //         }
                            //     }
                            // }
                            _ => {}
                        }
                    }
                    // Other ISO
                    if dst_cdrom_iso.is_some() {
                        copy_disks.push(CloneDisk {
                            // These should always have a value.
                            src: src_cdrom_iso.unwrap(),
                            dst: dst_cdrom_iso.clone().unwrap(),
                        })
                    }
                    if dst_usb_disk.is_some() {
                        copy_disks.push(CloneDisk {
                            // These should always have a value.
                            src: src_usb_disk.unwrap(),
                            dst: dst_usb_disk.clone().unwrap(),
                        })
                    }
                    let ignition_config = match device_model.name {
                        DeviceModels::FlatcarLinux => Some(true),
                        _ => None,
                    };

                    let domain = DomainTemplate {
                        qemu_bin: config.qemu_bin.clone(),
                        name: vm_name,
                        memory: device_model.memory,
                        cpu_architecture: device_model.cpu_architecture.clone(),
                        machine_type: device_model.machine_type.clone(),
                        cpu_count: device_model.cpu_count,
                        vmx_enabled: device_model.vmx_enabled,
                        bios: device_model.bios.clone(),
                        boot_disk: dst_boot_disk,
                        cdrom: dst_cdrom_iso,
                        usb_disk: dst_usb_disk,
                        ignition_config,
                        interfaces,
                        interface_type: device_model.interface_type.clone(),
                        loopback_ipv4: get_ip(device.id).to_string(),
                        telnet_port: TELNET_PORT,
                    };

                    domains.push(domain);
                }

                // Boot Server
                if config.ztp_server.enabled {
                    let boot_server_name = format!("{BOOT_SERVER_NAME}-{lab_id}");
                    let dir = format!("{TEMP_DIR}/{boot_server_name}");
                    let ignition_user = IgnitionUser {
                        name: user.username,
                        ssh_authorized_keys: vec![format!(
                            "{} {}",
                            user.ssh_public_key.algorithm, user.ssh_public_key.key
                        )],
                        groups: vec!["wheel".to_owned(), "docker".to_owned()],
                    };
                    let hostname_file = IgnitionFile {
                        // filesystem: "root".to_owned(),
                        path: "/etc/hostname".to_owned(),
                        mode: 644,
                        contents: IgnitionFileContents::new(&format!("data:,{BOOT_SERVER_NAME}")),
                    };
                    let unit_webdir = IgnitionUnit::default();
                    let unit_tftp = IgnitionUnit {
                        name: "tftpd.service".to_owned(),
                        enabled: true,
                        contents: r#"[Unit]
Description=TFTPd
After=docker.service
Requires=docker.service

[Service]
TimeoutStartSec=0
ExecStartPre=/usr/bin/docker image pull ghcr.io/bwks/tftpd:latest
ExecStart=/usr/bin/docker container run --rm --name tftpd-app -p 6969:6969/udp -v /opt/ztp:/opt/ztp ghcr.io/bwks/tftpd
ExecStop=/usr/bin/docker container stop tftpd-app

Restart=always
RestartSec=5s

[Install]
WantedBy=multi-user.target
"#.to_owned(),
                    };
                    // files
                    let sudo_config_base64 =
                        base64_encode(&format!("{SHERPA_USERNAME} ALL=(ALL) NOPASSWD: ALL"));
                    let sudo_config_file = IgnitionFile {
                        path: format!("/etc/sudoers.d/{SHERPA_USERNAME}"),
                        mode: 440,
                        contents: IgnitionFileContents::new(&format!(
                            "data:;base64,{sudo_config_base64}"
                        )),
                    };
                    let arista_ztp_base64 = base64_encode(&arista_ztp_script);
                    let arista_ztp_file = IgnitionFile {
                        // filesystem: "root".to_owned(),
                        path: format!("/opt/ztp/{ARISTA_ZTP_DIR}/{ARISTA_VEOS_ZTP_SCRIPT}"),
                        mode: 644,
                        contents: IgnitionFileContents::new(&format!(
                            "data:;base64,{arista_ztp_base64}"
                        )),
                    };
                    let aruba_ztp_base64 = base64_encode(&aruba_rendered_template);
                    let aruba_ztp_file = IgnitionFile {
                        // filesystem: "root".to_owned(),
                        path: format!("/opt/ztp/{ARUBA_ZTP_DIR}/{ARUBA_ZTP_CONFIG}"),
                        mode: 644,
                        contents: IgnitionFileContents::new(&format!(
                            "data:;base64,{aruba_ztp_base64}"
                        )),
                    };
                    let cumulus_ztp_base64 = base64_encode(&cumulus_rendered_template);
                    let cumulus_ztp_file = IgnitionFile {
                        // filesystem: "root".to_owned(),
                        path: format!("/opt/ztp/{CUMULUS_ZTP_DIR}/{CUMULUS_ZTP_CONFIG}"),
                        mode: 644,
                        contents: IgnitionFileContents::new(&format!(
                            "data:;base64,{cumulus_ztp_base64}"
                        )),
                    };
                    let iosxe_ztp_base64 = base64_encode(&iosxe_rendered_template);
                    let iosxe_ztp_file = IgnitionFile {
                        // filesystem: "root".to_owned(),
                        path: format!("/opt/ztp/{CISCO_ZTP_DIR}/{CISCO_IOSXE_ZTP_CONFIG}"),
                        mode: 644,
                        contents: IgnitionFileContents::new(&format!(
                            "data:;base64,{iosxe_ztp_base64}"
                        )),
                    };
                    let iosv_ztp_base64 = base64_encode(&iosv_rendered_template);
                    let iosv_ztp_file = IgnitionFile {
                        // filesystem: "root".to_owned(),
                        path: format!("/opt/ztp/{CISCO_ZTP_DIR}/{CISCO_IOSV_ZTP_CONFIG}"),
                        mode: 644,
                        contents: IgnitionFileContents::new(&format!(
                            "data:;base64,{iosv_ztp_base64}"
                        )),
                    };

                    let juniper_vjunos_ztp_base64 =
                        base64_encode(&juniper_vjunos_rendered_template);
                    let juniper_vjunos_ztp_file = IgnitionFile {
                        // filesystem: "root".to_owned(),
                        path: format!("/opt/ztp/{JUNIPER_ZTP_DIR}/{JUNIPER_ZTP_CONFIG}"),
                        mode: 644,
                        contents: IgnitionFileContents::new(&format!(
                            "data:;base64,{juniper_vjunos_ztp_base64}"
                        )),
                    };

                    let ignition_config = IgnitionConfig::new(
                        vec![ignition_user],
                        vec![
                            sudo_config_file,
                            hostname_file,
                            arista_ztp_file,
                            aruba_ztp_file,
                            cumulus_ztp_file,
                            iosxe_ztp_file,
                            iosv_ztp_file,
                            juniper_vjunos_ztp_file,
                        ],
                        vec![],
                        vec![unit_webdir, unit_tftp], // vec![link_default],
                    );
                    let flatcar_config = ignition_config.to_json_pretty()?;
                    let src_ztp_file = format!("{dir}/{ZTP_JSON}");
                    let dst_ztp_file = format!("{SHERPA_STORAGE_POOL_PATH}/{boot_server_name}.ign");

                    create_dir(&dir)?;
                    create_file(&src_ztp_file, flatcar_config)?;
                    copy_disks.push(CloneDisk {
                        src: src_ztp_file,
                        dst: dst_ztp_file.clone(),
                    });

                    let src_boot_disk = format!(
                        "{}/{}/{}/virtioa.qcow2",
                        sherpa.boxes_dir,
                        DeviceModels::FlatcarLinux,
                        "latest"
                    );
                    let dst_boot_disk =
                        format!("{SHERPA_STORAGE_POOL_PATH}/{boot_server_name}.qcow2");
                    copy_disks.push(CloneDisk {
                        src: src_boot_disk,
                        dst: dst_boot_disk.clone(),
                    });

                    let domain = DomainTemplate {
                        qemu_bin: config.qemu_bin.clone(),
                        name: boot_server_name.to_owned(),
                        memory: 512,
                        cpu_architecture: CpuArchitecture::default(),
                        machine_type: MachineTypes::default(),
                        cpu_count: 1,
                        vmx_enabled: false,
                        bios: BiosTypes::default(),
                        boot_disk: dst_boot_disk,
                        cdrom: None,
                        usb_disk: None,
                        ignition_config: Some(true),
                        interfaces: vec![Interface {
                            name: "mgmt".to_owned(),
                            num: 0,
                            mtu: MTU_JUMBO_INT,
                            mac_address: BOOT_SERVER_MAC.to_owned(),
                            connection_type: ConnectionTypes::Management,
                            connection_map: None,
                        }],
                        interface_type: InterfaceTypes::Virtio,
                        loopback_ipv4: get_ip(255).to_string(),
                        telnet_port: TELNET_PORT,
                    };

                    domains.push(domain);
                }

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
                            let rendered_xml = domain.render().with_context(|| {
                                format!("Failed to render XML for VM: {}", domain.name)
                            })?;
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

                // Check if VMs are ready
                term_msg_underline("Checking VM Readiness");
                let start_time = Instant::now();
                let timeout = Duration::from_secs(READINESS_TIMEOUT); // 10 minutes
                let mut connected_devices = std::collections::HashSet::new();
                let mut device_ip_map = vec![];
                let mut devices = manifest.devices;
                devices.push(Device {
                    id: 255,
                    name: BOOT_SERVER_NAME.to_owned(),
                    device_model: DeviceModels::FlatcarLinux,
                });

                while start_time.elapsed() < timeout && connected_devices.len() < devices.len() {
                    for device in &devices {
                        if connected_devices.contains(&device.name) {
                            continue;
                        }

                        let vm_name = format!("{}-{}", device.name, lab_id);
                        if let Some(vm_ip) = get_mgmt_ip(&qemu_conn, &vm_name)? {
                            match tcp_connect(&vm_ip, SSH_PORT)? {
                                true => {
                                    println!("{} is ready", &device.name);
                                    let ip = vm_ip;
                                    connected_devices.insert(device.name.clone());
                                    device_ip_map.push(DeviceIp {
                                        name: device.name.clone(),
                                        ip_address: ip,
                                    });
                                }
                                false => {
                                    println!("Waiting for {}", device.name);
                                }
                            }
                        } else {
                            println!("Waiting for {}", device.name);
                        }
                    }

                    if connected_devices.len() < devices.len() {
                        sleep(Duration::from_secs(READINESS_SLEEP));
                    }
                }

                if connected_devices.len() == devices.len() {
                    println!("All devices are ready!");
                } else {
                    println!("Timeout reached. Not all devices are ready.");
                    for device in &devices {
                        if !connected_devices.contains(&device.name) {
                            println!("Device is not ready: {}", device.name);
                        }
                    }
                }
                if !device_ip_map.is_empty() {
                    term_msg_underline("Creating SSH Config File");
                    let ssh_config_template = SshConfigTemplate {
                        hosts: device_ip_map,
                    };
                    let rendered_template = ssh_config_template.render()?;
                    create_file(
                        &format!("{TEMP_DIR}/{SHERPA_SSH_CONFIG_FILE}"),
                        rendered_template,
                    )?;
                }
            }
            Commands::Down => {
                term_msg_surround("Suspending environment");

                let lab_id = get_id()?;

                let qemu_conn = qemu.connect()?;

                let domains = qemu_conn.list_all_domains(0)?;

                for domain in domains {
                    let vm_name = domain.get_name()?;
                    if vm_name.contains(&lab_id) {
                        if domain.is_active()? {
                            domain.suspend()?;
                            println!("Suspended: {vm_name}");
                        } else {
                            println!("Virtual machine not running: {vm_name}");
                        }
                    }
                }
            }
            Commands::Resume => {
                term_msg_surround("Resuming environment");

                let lab_id = get_id()?;

                let qemu_conn = qemu.connect()?;

                let domains = qemu_conn.list_all_domains(0)?;

                for domain in domains {
                    let vm_name = domain.get_name()?;
                    if vm_name.contains(&lab_id) {
                        match domain.get_state() {
                            Ok((state, _reason)) => {
                                if state == virt::sys::VIR_DOMAIN_PAUSED {
                                    domain.resume()?;
                                    println!("Resumed: {vm_name}");
                                } else if state == virt::sys::VIR_DOMAIN_RUNNING {
                                    println!("Virtual machine already running: {vm_name}");
                                } else {
                                    println!(
                                        "Virtual machine not paused (state: {}): {}",
                                        state, vm_name
                                    );
                                }
                            }
                            Err(e) => anyhow::bail!("Failed to get state for {vm_name}: {e}"),
                        }
                    }
                }
            }
            Commands::Destroy => {
                term_msg_surround("Destroying environment");

                let lab_id = get_id()?;

                let qemu_conn = qemu.connect()?;

                let domains = qemu_conn.list_all_domains(0)?;

                for domain in domains {
                    let vm_name = domain.get_name()?;
                    if vm_name.contains(&lab_id) && domain.is_active()? {
                        // EUFI domains will have an NVRAM file that must be deleted.
                        let nvram_flag = sys::VIR_DOMAIN_UNDEFINE_NVRAM;
                        domain.undefine_flags(nvram_flag)?;
                        domain.destroy()?;
                        println!("Destroyed VM: {vm_name}");

                        // HDD
                        let hdd_name = format!("{vm_name}.qcow2");
                        delete_disk(&qemu_conn, &hdd_name)?;
                        println!("Deleted HDD: {hdd_name}");

                        // ISO
                        let iso_name = format!("{vm_name}.iso");
                        if file_exists(&format!("{SHERPA_STORAGE_POOL_PATH}/{iso_name}")) {
                            delete_disk(&qemu_conn, &iso_name)?;
                            println!("Deleted ISO: {iso_name}");
                        }

                        // Ignition
                        let ign_name = format!("{vm_name}.ign");
                        if file_exists(&format!("{SHERPA_STORAGE_POOL_PATH}/{ign_name}")) {
                            delete_disk(&qemu_conn, &ign_name)?;
                            println!("Deleted Ignition: {ign_name}");
                        }

                        // USB Image
                        let usb_name = format!("{vm_name}.img");
                        if file_exists(&format!("{SHERPA_STORAGE_POOL_PATH}/{usb_name}")) {
                            delete_disk(&qemu_conn, &usb_name)?;
                            println!("Deleted USB Disk: {usb_name}");
                        }
                    }
                }
                if dir_exists(TEMP_DIR) {
                    fs::remove_dir_all(TEMP_DIR)?;
                    println!("Deleted directory: {TEMP_DIR}");
                }
            }
            Commands::Inspect => {
                let lab_id = get_id()?;

                let manifest = Manifest::load_file()?;

                term_msg_surround(&format!("Sherpa Environment - {}", lab_id));

                let qemu_conn = qemu.connect()?;

                let domains = qemu_conn.list_all_domains(0)?;
                let pool = StoragePool::lookup_by_name(&qemu_conn, SHERPA_STORAGE_POOL)?;
                let mut devices = manifest.devices;
                devices.push(Device {
                    id: 255,
                    name: BOOT_SERVER_NAME.to_owned(),
                    device_model: DeviceModels::FlatcarLinux,
                });
                for device in devices {
                    let device_name = format!("{}-{}", device.name, lab_id);
                    if let Some(domain) = domains
                        .iter()
                        .find(|d| d.get_name().unwrap_or_default() == device_name)
                    {
                        term_msg_underline(&device.name);
                        println!("Domain: {}", device_name);
                        println!("Model: {}", device.device_model);
                        println!("Active: {:#?}", domain.is_active()?);
                        if let Some(vm_ip) = get_mgmt_ip(&qemu_conn, &device_name)? {
                            println!("Mgmt IP: {vm_ip}");
                        }
                        for volume in pool.list_volumes()? {
                            if volume.contains(&device_name) {
                                println!("Disk: {volume}");
                            }
                        }
                    }
                }
            }
            Commands::Import {
                src,
                version,
                model,
                latest,
            } => {
                term_msg_surround("Importing disk image");

                if !file_exists(src) {
                    anyhow::bail!("File does not exist: {}", src);
                }

                let dst_path = format!("{}/{}", sherpa.boxes_dir, model);
                let dst_version_dir = format!("{dst_path}/{version}");
                let dst_latest_dir = format!("{dst_path}/latest");

                create_dir(&dst_version_dir)?;
                create_dir(&dst_latest_dir)?;

                let dst_version_disk = format!("{dst_version_dir}/virtioa.qcow2");

                if !file_exists(&dst_version_disk) {
                    println!("Copying file from: {} to: {}", src, dst_version_disk);
                    copy_file(src, &dst_version_disk)?;
                    println!("Copied file from: {} to: {}", src, dst_version_disk);
                } else {
                    println!("File already exists: {}", dst_version_disk);
                }

                if *latest {
                    let dst_latest_disk = format!("{dst_latest_dir}/virtioa.qcow2");
                    println!("Copying file from: {} to: {}", src, dst_latest_disk);
                    copy_file(src, &dst_latest_disk)?;
                    println!("Copied file from: {} to: {}", src, dst_latest_disk);
                }

                println!("Setting base box files to read-only");
                fix_permissions_recursive(&format!("{SHERPA_CONFIG_DIR}/{SHERPA_BOXES_DIR}"))?;
            }
            Commands::Doctor { boxes } => {
                if *boxes {
                    term_msg_surround("Fixing base box permissions");

                    fix_permissions_recursive(&format!("{SHERPA_CONFIG_DIR}/{SHERPA_BOXES_DIR}"))?;
                }
            }
            Commands::Clean {
                all,
                disks,
                networks,
            } => {
                if *all {
                    // term_msg_surround("Cleaning environment");
                    term_msg_surround("Not implemented");
                } else if *disks {
                    term_msg_surround("Cleaning disks");
                    let lab_id = get_id()?;

                    let qemu_conn = qemu.connect()?;

                    let pool = StoragePool::lookup_by_name(&qemu_conn, SHERPA_STORAGE_POOL)?;
                    for volume in pool.list_volumes()? {
                        if volume.contains(&lab_id) {
                            println!("Deleting disk: {}", volume);
                            let vol = StorageVol::lookup_by_name(&pool, &volume)?;
                            vol.delete(0)?;
                            println!("Deleted disk: {}", volume);
                        }
                    }
                } else if *networks {
                    term_msg_surround("Cleaning networks");

                    let qemu_conn = qemu.connect()?;

                    let networks = qemu_conn.list_all_networks(0)?;
                    for network in networks {
                        if network.get_name()?.contains("sherpa") {
                            let network_name = network.get_name()?;
                            println!("Destroying network: {}", network_name);
                            network.destroy()?;
                            network.undefine()?;
                            println!("Destroyed network: {}", network_name);
                        }
                    }
                }
            }
            Commands::Console { name } => {
                term_msg_surround(&format!("Connecting to: {name}"));

                let manifest = Manifest::load_file()?;

                // Find the device in the manifest
                let device_ip = {
                    if name == BOOT_SERVER_NAME {
                        get_ip(255)
                    } else {
                        let device = manifest
                            .devices
                            .iter()
                            .find(|d| d.name == *name)
                            .ok_or_else(|| anyhow::anyhow!("Device not found: {}", name))?;
                        get_ip(device.id)
                    }
                };

                let status = Command::new("telnet")
                    .arg(device_ip.to_string())
                    .arg(TELNET_PORT.to_string())
                    .status()?;

                if !status.success() {
                    eprintln!("Telnet connection failed");
                    if let Some(code) = status.code() {
                        eprintln!("Exit code: {}", code);
                    }
                }
            }
            Commands::Ssh { name } => {
                term_msg_surround(&format!("Connecting to: {name}"));
                let lab_id = get_id()?;

                let qemu_conn = qemu.connect()?;

                if let Some(vm_ip) = get_mgmt_ip(&qemu_conn, &format!("{}-{}", name, lab_id))? {
                    let status = Command::new("ssh")
                        .arg(&vm_ip)
                        .arg("-F")
                        .arg(format!("{TEMP_DIR}/{SHERPA_SSH_CONFIG_FILE}"))
                        .status()?;

                    if !status.success() {
                        eprintln!("SSH connection failed");
                        if let Some(code) = status.code() {
                            eprintln!("Exit code: {}", code);
                        }
                    }
                } else {
                    eprintln!("No IP address found for {name}")
                }
            }
        }
        Ok(())
    }
}
