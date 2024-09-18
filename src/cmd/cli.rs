use std::collections::HashMap;
use std::process::Command;

use anyhow::Result;

use askama::Template;

use clap::{Parser, Subcommand};

use crate::core::konst::{
    BOOT_NETWORK_BRIDGE, BOOT_NETWORK_DHCP_END, BOOT_NETWORK_DHCP_START, BOOT_NETWORK_IP,
    BOOT_NETWORK_NAME, BOOT_NETWORK_NETMASK, CONFIG_FILE, MANIFEST_FILE, STORAGE_POOL_PATH,
    TELNET_PORT,
};
use crate::core::{Config, Sherpa};
use crate::libvirt::{
    clone_disk, create_isolated_network, create_network, create_vm, delete_disk, DomainTemplate,
    Qemu,
};
use crate::model::{ConnectionTypes, DeviceModel, Interface};
use crate::topology::{ConnectionMap, Manifest};
use crate::util::{create_dir, dir_exists, file_exists, get_ip, id_to_port, random_mac, term_msg};

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
        #[arg(default_value = CONFIG_FILE)]
        config_file: String,

        /// Name of the manifest file
        #[arg(default_value = MANIFEST_FILE)]
        manifest_file: String,

        /// Overwrite config file if one exists
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        force: bool,
    },
    /// Build environment
    Up {
        /// Name of the config file
        #[arg(default_value = CONFIG_FILE)]
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

    /// Connect to a device
    Connect { name: String },

    /// Create an isolated bridge
    CreateBridge { name: String },
}
impl Default for Commands {
    fn default() -> Self {
        Commands::Init {
            config_file: CONFIG_FILE.to_owned(),
            manifest_file: MANIFEST_FILE.to_owned(),
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
                term_msg("Sherpa Initializing");
                let qemu_conn = qemu.connect()?;

                sherpa.config_path = format!("{}/{}", sherpa.config_dir, config_file);

                // Create the default config directories
                if dir_exists(&sherpa.config_dir) && !*force {
                    println!("Directory path already exists: {}", sherpa.config_dir);
                } else {
                    create_dir(&sherpa.config_dir)?;
                    create_dir(&sherpa.boxes_dir)?;
                    // box directories
                    let config = Config::default();
                    for device_model in config.device_models {
                        create_dir(&format!(
                            "{}/{}/latest",
                            sherpa.boxes_dir, device_model.name
                        ))?;
                    }
                }

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

                // Initialize the network
                if qemu_conn
                    .list_networks()?
                    .iter()
                    .any(|net| net == "sherpanet")
                {
                    println!("Network already exists: sherpanet");
                } else {
                    println!("Creating network: sherpanet");
                    create_network(
                        &qemu_conn,
                        BOOT_NETWORK_NAME,
                        BOOT_NETWORK_BRIDGE,
                        BOOT_NETWORK_IP,
                        BOOT_NETWORK_NETMASK,
                        BOOT_NETWORK_DHCP_START,
                        BOOT_NETWORK_DHCP_END,
                    )?;
                }
            }
            Commands::Up { config_file } => {
                term_msg("Building environment");

                let qemu_conn = qemu.connect()?;

                sherpa.config_path = format!("{}/{}", sherpa.config_dir, config_file);

                let config = Config::load(&sherpa.config_path)?;
                let manifest = Manifest::load_file()?;

                // Create a mapping of device name to device id.
                let dev_id_map: HashMap<String, u8> = manifest
                    .devices
                    .iter()
                    .map(|d| (d.name.clone(), d.id))
                    .collect();

                let mut domains: Vec<DomainTemplate> = vec![];
                for device in manifest.devices {
                    let device_model = DeviceModel::get_model(device.device_model);

                    let mut interfaces: Vec<Interface> = vec![];

                    // Build interface vector.
                    for i in 0..device_model.interface_count {
                        for c in &manifest.connections {
                            // Device is source in manifest
                            if c.device_a == device.name && i == c.interface_a {
                                let source_id = dev_id_map.get(&c.device_b).ok_or_else(|| {
                                    anyhow::anyhow!("Connection device_b not found: {}", c.device_b)
                                })?;
                                let connection_map = ConnectionMap {
                                    local_id: device.id,
                                    local_port: id_to_port(i),
                                    local_loopback: get_ip(device.id).to_string(),
                                    source_id: source_id.to_owned(),
                                    source_port: id_to_port(c.interface_b),
                                    source_loopback: get_ip(source_id.to_owned()).to_string(),
                                };
                                interfaces.push(Interface {
                                    name: format!("{}{}", device_model.interface_prefix, i),
                                    num: i,
                                    mac_address: random_mac(),
                                    connection_type: ConnectionTypes::Peer,
                                    connection_map: Some(connection_map),
                                })
                            // Device is destination in manifest
                            } else if c.device_b == device.name && i == c.interface_b {
                                let source_id = dev_id_map.get(&c.device_a).ok_or_else(|| {
                                    anyhow::anyhow!("Connection device_a not found: {}", c.device_a)
                                })?;
                                let connection_map = ConnectionMap {
                                    local_id: device.id,
                                    local_port: id_to_port(i),
                                    local_loopback: get_ip(device.id).to_string(),
                                    source_id: source_id.to_owned(),
                                    source_port: id_to_port(c.interface_a),
                                    source_loopback: get_ip(source_id.to_owned()).to_string(),
                                };
                                interfaces.push(Interface {
                                    name: format!("{}{}", device_model.interface_prefix, i),
                                    num: i,
                                    mac_address: random_mac(),
                                    connection_type: ConnectionTypes::Peer,
                                    connection_map: Some(connection_map),
                                })
                            } else {
                                // Interface not defined in manifest so disable.
                                interfaces.push(Interface {
                                    name: format!("{}{}", device_model.interface_prefix, i),
                                    num: i,
                                    mac_address: random_mac(),
                                    connection_type: ConnectionTypes::Disabled,
                                    connection_map: None,
                                })
                            }
                        }
                    }

                    let vm_name = format!("{}-{}", device.name, manifest.id);
                    let src_boot_disk = format!(
                        "{}/{}/{}/virtioa.qcow2",
                        sherpa.boxes_dir, device_model.name, device_model.version
                    );
                    let dst_boot_disk = format!("{STORAGE_POOL_PATH}/{vm_name}.qcow2");

                    clone_disk(&qemu_conn, &src_boot_disk, &dst_boot_disk)?;

                    // CDROM ISO
                    let dst_cdrom_iso = match device_model.cdrom_iso {
                        Some(src_iso) => {
                            let src = format!(
                                "{}/{}/{}/{}",
                                sherpa.boxes_dir, device_model.name, device_model.version, src_iso
                            );
                            let dst = format!("{STORAGE_POOL_PATH}/{vm_name}.iso");
                            clone_disk(&qemu_conn, &src, &dst)?;
                            Some(dst)
                        }
                        None => None,
                    };

                    let domain = DomainTemplate {
                        name: vm_name,
                        memory: device_model.memory,
                        cpu_architecture: device_model.cpu_architecture,
                        machine_type: device_model.machine_type,
                        cpu_count: device_model.cpu_count,
                        qemu_bin: config.qemu_bin.clone(),
                        boot_disk: dst_boot_disk,
                        cdrom_iso: dst_cdrom_iso,
                        interfaces,
                        interface_type: device_model.interface_type,
                        loopback_ipv4: get_ip(device.id).to_string(),
                        telnet_port: TELNET_PORT,
                    };

                    domains.push(domain);
                }

                // Build domains
                for domain in domains {
                    let rendered_xml = domain.render()?;

                    let xml_configs = vec![rendered_xml];

                    for xml in xml_configs {
                        let result = create_vm(&qemu_conn, &xml);
                        match result {
                            Ok(_) => println!("Created: {}", domain.name),
                            Err(_) => eprintln!("Create failed: {}", domain.name),
                        }
                    }
                }
            }
            Commands::Down => {
                term_msg("Suspending environment");

                let manifest = Manifest::load_file()?;

                let qemu_conn = qemu.connect()?;

                let domains = qemu_conn.list_all_domains(0)?;

                for domain in domains {
                    let vm_name = domain.get_name()?;
                    if vm_name.contains(&manifest.id) {
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
                term_msg("Resuming environment");

                let manifest = Manifest::load_file()?;

                let qemu_conn = qemu.connect()?;

                let domains = qemu_conn.list_all_domains(0)?;

                for domain in domains {
                    let vm_name = domain.get_name()?;
                    if vm_name.contains(&manifest.id) {
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
                term_msg("Destroying environment");

                let manifest = Manifest::load_file()?;

                let qemu_conn = qemu.connect()?;

                let domains = qemu_conn.list_all_domains(0)?;

                for domain in domains {
                    let vm_name = domain.get_name()?;
                    if vm_name.contains(&manifest.id) && domain.is_active()? {
                        domain.destroy()?;
                        println!("Destroyed: {vm_name}");

                        // HDD
                        let hdd_name = format!("{vm_name}.qcow2");
                        delete_disk(&qemu_conn, &hdd_name)?;
                        println!("Deleted HDD: {hdd_name}");

                        // ISO
                        let iso_name = format!("{vm_name}.iso");
                        if file_exists(&format!("{STORAGE_POOL_PATH}/{iso_name}")) {
                            delete_disk(&qemu_conn, &iso_name)?;
                            println!("Deleted ISO: {iso_name}");
                        }
                    }
                }
            }
            Commands::Inspect => {
                term_msg("Sherpa Environemnt");

                let manifest = Manifest::load_file()?;

                let qemu_conn = qemu.connect()?;

                let domains = qemu_conn.list_all_domains(0)?;
                for domain in domains {
                    let vm_name = domain.get_name()?;
                    if vm_name.contains(&manifest.id) {
                        println!("VM: {vm_name}");
                    }
                }
            }
            Commands::Connect { name } => {
                term_msg(&format!("Connecting to: {name}"));

                let manifest = Manifest::load_file()?;

                // Find the device in the manifest
                let device = manifest
                    .devices
                    .iter()
                    .find(|d| d.name == *name)
                    .ok_or_else(|| anyhow::anyhow!("Device not found: {}", name))?;

                let status = Command::new("telnet")
                    .arg(get_ip(device.id).to_string())
                    .arg(TELNET_PORT.to_string())
                    .status()?;

                if !status.success() {
                    eprintln!("Telnet connection failed");
                    if let Some(code) = status.code() {
                        eprintln!("Exit code: {}", code);
                    }
                }
            }
            Commands::CreateBridge { name } => {
                term_msg(&format!("Creating bridge: {name}"));
                let qemu_conn = qemu.connect()?;
                create_isolated_network(&qemu_conn, name)
                    .map_err(|e| anyhow::anyhow!("Failed to create bridge: {}", e))?;
            }
        }
        Ok(())
    }
}
