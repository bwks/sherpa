use std::collections::HashMap;
use std::hash::Hash;

use anyhow::Result;
use askama::Template;
use clap::{Parser, Subcommand};

use virt::connect::Connect;
use virt::domain::Domain;

use crate::core::konst::{CONFIG_FILE, MANIFEST_FILE};
use crate::core::{Config, Sherpa};
use crate::libvirt::DomainTemplate;
use crate::libvirt::Qemu;
use crate::model::{ConnectionTypes, DeviceModel, Interface};
use crate::topology::{ConnectionMap, Manifest};
use crate::util::{
    copy_file, create_dir, delete_file, dir_exists, file_exists, get_ip, id_to_port, random_mac,
    term_msg,
};

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
    /// Destroy environment
    Destroy,
    /// Inspect environment
    Inspect,

    /// Connect to a device
    Connect { name: String },
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

                sherpa.config_path = format!("{}/{}", sherpa.config_dir, config_file);

                // Create the default config directories
                if dir_exists(sherpa.config_dir.as_str()) && !*force {
                    println!("Directory path already exists: {}", sherpa.config_dir);
                } else {
                    create_dir(&sherpa.config_dir)?;
                    create_dir(&sherpa.boxes_dir)?
                }

                // Initialize default files
                if file_exists(&sherpa.config_path) && !*force {
                    println!("Config file already exists: {}", sherpa.config_path);
                } else {
                    let mut config = Config::default();
                    config.name = config_file.to_owned();
                    config.create(&sherpa.config_path)?;
                }

                if file_exists(&manifest_file) && !*force {
                    println!("Manifest file already exists: {manifest_file}");
                } else {
                    let manifest = Manifest::default();
                    manifest.write_file()?;
                }
            }
            Commands::Up { config_file } => {
                term_msg("Building environment");

                let qemu_conn = qemu.connect()?;

                sherpa.config_path = format!("{}/{}", sherpa.config_dir, config_file);

                let config = Config::load(&sherpa.config_path)?;
                let manifest = Manifest::load_file()?;

                let dev_id_map: HashMap<String, u8> = manifest
                    .devices
                    .iter()
                    .map(|d| (d.name.clone(), d.id))
                    .collect();

                let mut domains: Vec<DomainTemplate> = vec![];
                for device in manifest.devices {
                    let device_model = DeviceModel::get_model(device.device_model);

                    let mut interfaces: Vec<Interface> = vec![];

                    for i in 0..device_model.interface_count {
                        for c in &manifest.connections {
                            if c.device_a == device.name && i == c.interface_a {
                                // Device is source in manifest
                                let connection_map = ConnectionMap {
                                    local_id: device.id,
                                    local_port: id_to_port(i),
                                    local_loopback: get_ip(device.id).to_string(),
                                    source_id: dev_id_map.get(&c.device_b).unwrap().to_owned(),
                                    source_port: id_to_port(c.interface_b),
                                    source_loopback: get_ip(
                                        dev_id_map.get(&c.device_b).unwrap().to_owned(),
                                    )
                                    .to_string(),
                                };
                                interfaces.push(Interface {
                                    name: format!("{}{}", device_model.interface_prefix, i),
                                    num: i,
                                    mac_address: format!("{}", random_mac()).to_owned(),
                                    connection_type: ConnectionTypes::Peer,
                                    connection_map: Some(connection_map),
                                })
                            } else if c.device_b == device.name && i == c.interface_b {
                                // Device is destination in manifest
                                let connection_map = ConnectionMap {
                                    local_id: device.id,
                                    local_port: id_to_port(i),
                                    local_loopback: get_ip(device.id).to_string(),
                                    source_id: dev_id_map.get(&c.device_a).unwrap().to_owned(),
                                    source_port: id_to_port(c.interface_a),
                                    source_loopback: get_ip(
                                        dev_id_map.get(&c.device_a).unwrap().to_owned(),
                                    )
                                    .to_string(),
                                };
                                interfaces.push(Interface {
                                    name: format!("{}{}", device_model.interface_prefix, i),
                                    num: i,
                                    mac_address: format!("{}", random_mac()).to_owned(),
                                    connection_type: ConnectionTypes::Peer,
                                    connection_map: Some(connection_map),
                                })
                            } else {
                                // Interface not defined in manifest so disable.
                                interfaces.push(Interface {
                                    name: format!("{}{}", device_model.interface_prefix, i),
                                    num: i,
                                    mac_address: format!("{}", random_mac()).to_owned(),
                                    connection_type: ConnectionTypes::Disabled,
                                    connection_map: None,
                                })
                            }
                        }
                    }

                    let vm_name = format!("{}-{}", device.name, manifest.id);
                    let src_file = format!(
                        "{}/{}/virtioa.qcow2",
                        sherpa.boxes_dir, device_model.version
                    );
                    let dst_file = format!("/tmp/{}.qcow2", vm_name);

                    copy_file(src_file.as_str(), dst_file.as_str())?;

                    println!("{:#?}", interfaces);

                    let domain = DomainTemplate {
                        id: device.id,
                        name: vm_name,
                        memory: device_model.memory,
                        cpu_architecture: device_model.cpu_architecture,
                        machine_type: device_model.machine_type,
                        cpu_count: device_model.cpu_count,
                        qemu_bin: config.qemu_bin.clone(),
                        boot_disk: dst_file,
                        interfaces,
                        interface_type: device_model.interface_type,
                    };

                    domains.push(domain);
                }

                // Build domains
                for domain in domains {
                    // Render the XML document
                    let rendered_xml = domain.render().unwrap();

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
                // TODO: Update this to stop the vm.
                term_msg("Stopping environment");

                let manifest = Manifest::load_file()?;

                let qemu_conn = qemu.connect()?;

                let domains = qemu_conn.list_all_domains(0).unwrap();

                for domain in domains {
                    let vm_name = domain.get_name()?;
                    if vm_name.contains(&manifest.id) {
                        if domain.is_active().unwrap_or(false) {
                            match domain.suspend() {
                                Ok(_) => println!("Shutdown: {vm_name}"),
                                Err(_) => eprintln!("Shutdown failed: {vm_name}"), // TODO: Raise
                            }
                        } else {
                            println!("Virtual machine not running: {vm_name}");
                        }
                    }
                }
            }
            Commands::Destroy => {
                term_msg("Destroying environment");

                let manifest = Manifest::load_file()?;

                let qemu_conn = qemu.connect()?;

                let domains = qemu_conn.list_all_domains(0).unwrap();

                for domain in domains {
                    let vm_name = domain.get_name()?;
                    if vm_name.contains(&manifest.id) {
                        if domain.is_active().unwrap_or(false) {
                            match domain.destroy() {
                                Ok(_) => println!("Destroyed: {vm_name}"),
                                Err(_) => eprintln!("Destroy failed: {vm_name}"), // TODO: Raise
                            }

                            let file_path = format!("/tmp/{}.qcow2", vm_name);
                            delete_file(&file_path)?;
                        }
                    }
                }
            }
            Commands::Inspect => {
                term_msg("Sherpa Environemnt");

                let manifest = Manifest::load_file()?;

                let qemu_conn = qemu.connect()?;

                let domains = qemu_conn.list_all_domains(0).unwrap();
                for domain in domains {
                    let vm_name = domain.get_name()?;
                    if vm_name.contains(&manifest.id) {
                        println!("VM: {vm_name}");
                    }
                }
            }
            Commands::Connect { name } => {
                term_msg(format!("Connecting to: {name}").as_str());

                let manifest = Manifest::load_file()?;

                let qemu_conn = qemu.connect()?;

                let vm_name = format!("{}-{}", name, manifest.id);

                // Get the domain (VM) by name
                let domain = Domain::lookup_by_name(&qemu_conn, &vm_name)?;
                if domain.is_active()? {
                    println!("Connecting to: {name}")
                } else {
                    println!("Device not found: {name}")
                }
            }
        }

        Ok(())
    }
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

// Create a virtual machine
fn create_vm(conn: &Connect, xml: &str) -> Result<Domain> {
    let domain = Domain::create_xml(conn, xml, 0)?;
    Ok(domain)
}
