use std::fs;

use anyhow::Result;
use askama::Template;
use clap::{Parser, Subcommand};

use crate::core::konst::{CONFIG_FILENAME, KVM_OUI, QEMU_BIN};
use crate::core::Config;
use crate::libvirt::DomainTemplate;
use crate::libvirt::Qemu;
use crate::model::{
    CpuArchitecture, DeviceModel, DeviceModels, InterfaceTypes, MachineTypes, Manufacturers,
    OsVariants,
};
use crate::topology::Manifest;
use crate::util::random_mac_suffix;

use virt::domain::Domain;

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
        #[arg(default_value = CONFIG_FILENAME)]
        config_file: String,
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
        let mut config = Config::default();
        let manifest = Manifest::default();

        match &cli.commands {
            Commands::Init { config_file } => {
                config.name = config_file.to_owned();
                println!("Initializing with config file: {:#?}", config.name);
                config.write_file()?;
                manifest.write_file()?;
            }
            Commands::Up => {
                println!("Building environment");
                manifest.load_file()?;

                let device: DeviceModel = DeviceModel {
                    name: DeviceModels::CiscoIosv,
                    os_variant: OsVariants::Ios,
                    manufacturer: Manufacturers::Cisco,
                    interface_count: 8,
                    interface_prefix: "Gig0/".to_owned(),
                    interface_type: InterfaceTypes::E1000,
                    cpu_count: 1,
                    cpu_architecture: CpuArchitecture::X86_64,
                    machine_type: MachineTypes::Pc_Q35_6_2,
                    memory: 1024,
                };

                let template = DomainTemplate {
                    name: "iosv".to_owned(),
                    cpu_count: device.cpu_count,
                    cpu_architecture: device.cpu_architecture,
                    machine_type: device.machine_type,
                    memory: device.memory,
                    qemu_bin: QEMU_BIN.to_owned(),
                    boot_disk: "/home/bradmin/Documents/code/rust/sherpa/vios-adventerprisek9-m.SPA.159-3.M6/virtioa.qcow2".to_owned(),
                    mac_address: format!("{}:{}", KVM_OUI, random_mac_suffix()).to_owned(),
                };

                // Render the XML document
                let rendered_xml = template.render().unwrap();
                println!("{}", rendered_xml);

                let qemu = Qemu::default();
                let qemu_conn = qemu.connect()?;

                // Define the domain (VM) using the XML data
                let domain = Domain::define_xml(&qemu_conn, &rendered_xml)
                    .expect("Failed to define the domain");

                println!("Domain defined: {:?}", domain.get_name().unwrap());

                // Optionally start the VM
                domain.create().expect("Failed to start the domain");
                println!("Domain started: {:?}", domain.get_name().unwrap());
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
        }
    }
}
