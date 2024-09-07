use anyhow::Result;
use askama::Template;
use clap::{Parser, Subcommand};

use virt::connect::Connect;
use virt::domain::Domain;

use crate::core::konst::{BOXES_DIR, CONFIG_DIR, CONFIG_FILE, KVM_OUI, MANIFEST_FILE};
use crate::core::Config;
use crate::libvirt::DomainTemplate;
use crate::libvirt::Qemu;
use crate::model::{DeviceModel, Interface};
use crate::topology::Manifest;
use crate::util::{create_dir, dir_exists, expand_path, file_exists, random_mac_suffix, term_msg};

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
    Up,
    /// Stop environment
    Down,
    /// Destroy environment
    Destroy,
    /// Inspect environment
    Inspect,
}

impl Cli {
    pub fn run() -> Result<()> {
        let cli = Cli::parse();

        match &cli.commands {
            Commands::Init {
                config_file,
                manifest_file,
                force,
            } => {
                term_msg("Sherpa Initializing");

                // Create the default config directories
                let config_dir = expand_path(format!("{CONFIG_DIR}").as_str());
                let boxes_dir = format!("{config_dir}/{BOXES_DIR}");
                let config_path = expand_path(format!("{CONFIG_DIR}/{config_file}").as_str());

                if dir_exists(config_dir.as_str()) && !*force {
                    println!("Directory path already exists: {config_dir}");
                } else {
                    create_dir(&config_dir)?;
                    create_dir(&boxes_dir)?
                }

                // Initialize default files
                if file_exists(&config_path) && !*force {
                    println!("Config file already exists: {config_path}");
                } else {
                    let mut config = Config::default();
                    config.name = config_file.to_owned();
                    config.create(&config_path)?;
                }

                if file_exists(&manifest_file) && !*force {
                    println!("Manifest file already exists: {manifest_file}");
                } else {
                    let manifest = Manifest::default();
                    manifest.write_file()?;
                }
            }
            Commands::Up => {
                println!("Building environment");
                let config = Config::load(&format!("{CONFIG_DIR}/{CONFIG_FILE}"))?;
                let manifest = Manifest::load_file()?;

                let mut domains: Vec<DomainTemplate> = vec![];
                for device in manifest.devices {
                    let device_model = DeviceModel::get_model(device.device_model);

                    let mut interfaces: Vec<Interface> = vec![];
                    for i in 0..device_model.interface_count {
                        interfaces.push(Interface {
                            name: format!("{}/{}", device_model.interface_prefix, i),
                            num: i,
                            mac_address: format!("{}:{}", KVM_OUI, random_mac_suffix()).to_owned(),
                        })
                    }

                    let domain = DomainTemplate {
                        name: device.name,
                        memory: device_model.memory,
                        cpu_architecture: device_model.cpu_architecture,
                        machine_type: device_model.machine_type,
                        cpu_count: device_model.cpu_count,
                        qemu_bin: config.qemu_bin.clone(),
                        boot_disk: "/tmp/vios-adventerprisek9-m.SPA.159-3.M6/virtioa.qcow2"
                            .to_owned(),
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
                    // println!("{}", rendered_xml);

                    let qemu = Qemu::default();
                    let qemu_conn = qemu.connect()?;

                    for xml in xml_configs {
                        let result = create_vm(&qemu_conn, &xml);
                        match result {
                            Ok(domain) => println!("Created VM: {}", domain.get_name()?),
                            Err(e) => eprintln!("Failed to create VM: {}", e),
                        }
                    }
                }
            }
            Commands::Down => {
                println!("Stopping environment");

                let qemu = Qemu::default();
                let qemu_conn = qemu.connect()?;
                let vm_name = "iosv";
                let domains = qemu_conn.list_all_domains(0).unwrap();
                for domain in domains {
                    println!("VM Name: {:?}", domain.get_name().unwrap());
                    if domain.get_name()? == vm_name {
                        println!("VM XML: {:?}", domain.get_xml_desc(0));
                        // Destroy the VM if it is running
                        if domain.is_active().unwrap_or(false) {
                            domain.destroy().expect("Failed to destroy the VM");
                            println!("VM '{}' has been destroyed", vm_name);
                        } else {
                            println!("VM '{}' is not running", vm_name);
                        }
                    }
                }
            }
            Commands::Destroy => {
                println!("Destroying environment");

                let qemu = Qemu::default();
                let qemu_conn = qemu.connect()?;
                let vm_name = "iosv";
                let domains = qemu_conn.list_all_domains(0).unwrap();
                for domain in domains {
                    println!("VM Name: {:?}", domain.get_name().unwrap());
                    if domain.get_name()? == vm_name {
                        println!("VM XML: {:?}", domain.get_xml_desc(0));
                        // Destroy the VM if it is running
                        if !domain.is_active().unwrap_or(false) {
                            // Undefine the VM, removing it from libvirt
                            domain.undefine().expect("Failed to undefine the VM");
                            println!("VM '{}' has been undefined", vm_name);
                        } else {
                            println!("VM '{}' is running", vm_name);
                        }
                    }
                }
            }
            Commands::Inspect => {
                let qemu = Qemu::default();
                let qemu_conn = qemu.connect()?;

                let domains = qemu_conn.list_all_domains(0).unwrap();
                for domain in domains {
                    println!("VM Name: {:?}", domain.get_name().unwrap());
                    if domain.get_name()? == "iosv" {
                        println!("VM XML: {:?}", domain.get_xml_desc(0));
                    }
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

fn create_vm(conn: &Connect, xml: &str) -> Result<Domain> {
    let domain = Domain::create_xml(conn, xml, 0)?;
    println!("Domain started: {:?}", domain.get_name().unwrap());
    Ok(domain)
}
