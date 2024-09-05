use std::path::Path;

use anyhow::Result;
use askama::Template;
use clap::{Parser, Subcommand};

use virt::connect::Connect;
use virt::domain::Domain;

use crate::core::konst::{CONFIG_FILENAME, KVM_OUI, MANIFEST_FILENAME, QEMU_BIN};
use crate::core::Config;
use crate::libvirt::DomainTemplate;
use crate::libvirt::Qemu;
use crate::model::{
    CpuArchitecture, DeviceModel, DeviceModels, Interface, InterfaceTypes, MachineTypes,
    Manufacturers, OsVariants,
};
use crate::topology::Manifest;
use crate::util::random_mac_suffix;

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
        #[arg(default_value = CONFIG_FILENAME)]
        config_file: String,

        /// Name of the manifest file
        #[arg(default_value = MANIFEST_FILENAME)]
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
                let mut init_config = true;
                let mut init_manifest = true;

                if Path::new(config_file).exists() && !*force {
                    println!("Config file: '{config_file}' already exists.");
                    init_config = false;
                }
                if Path::new(manifest_file).exists() && !*force {
                    println!("Manifest file: '{manifest_file}' already exists.");
                    init_manifest = false;
                }
                if init_config {
                    println!("Initializing config file: '{config_file}'");
                    let mut config = Config::default();
                    config.name = config_file.to_owned();
                    config.write_file()?;
                }
                if init_manifest {
                    println!("Initializing manifest file: '{manifest_file}'");
                    let manifest = Manifest::default();
                    manifest.write_file()?;
                }
            }
            Commands::Up => {
                println!("Building environment");
                let _config = Config::load_file();
                let _manifest = Manifest::load_file();

                let device: DeviceModel = DeviceModel {
                    name: DeviceModels::CiscoIosv,
                    os_variant: OsVariants::Ios,
                    manufacturer: Manufacturers::Cisco,
                    interface_count: 8,
                    interface_prefix: "Gig0/".to_owned(),
                    interface_type: InterfaceTypes::E1000,
                    cpu_count: 1,
                    cpu_architecture: CpuArchitecture::X86_64,
                    machine_type: MachineTypes::PcQ35_6_2,
                    memory: 1024,
                };
                let mut interfaces: Vec<Interface> = vec![];
                for i in 0..device.interface_count {
                    interfaces.push(Interface {
                        name: format!("{}/{}", device.interface_prefix, i),
                        num: i,
                        mac_address: format!("{}:{}", KVM_OUI, random_mac_suffix()).to_owned(),
                    })
                }

                let template1 = DomainTemplate {
                    name: "iosv1".to_owned(),
                    cpu_count: device.cpu_count,
                    cpu_architecture: device.cpu_architecture,
                    machine_type: device.machine_type,
                    memory: device.memory,
                    qemu_bin: QEMU_BIN.to_owned(),
                    boot_disk: "/tmp/vios-adventerprisek9-m.SPA.159-3.M6/virtioa.qcow2".to_owned(),
                    interfaces: interfaces,
                    interface_type: device.interface_type,
                };

                // Render the XML document
                let rendered_xml = template1.render().unwrap();

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
            config_file: CONFIG_FILENAME.to_owned(),
            manifest_file: MANIFEST_FILENAME.to_owned(),
            force: false,
        }
    }
}

fn create_vm(conn: &Connect, xml: &str) -> Result<Domain> {
    let domain = Domain::create_xml(conn, xml, 0)?;
    println!("Domain started: {:?}", domain.get_name().unwrap());
    Ok(domain)
}
