use std::fs;

use anyhow::Result;
use askama::Template;
use clap::{Parser, Subcommand};

use futures::future::join_all;
use tokio;
use virt::connect::Connect;
use virt::domain::Domain;

use crate::core::konst::{CONFIG_FILENAME, KVM_OUI, QEMU_BIN};
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

                let device1: DeviceModel = DeviceModel {
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
                let mut interfaces1: Vec<Interface> = vec![];
                for i in 0..device1.interface_count {
                    interfaces1.push(Interface {
                        name: format!("{}/{}", device1.interface_prefix, i),
                        num: i,
                        mac_address: format!("{}:{}", KVM_OUI, random_mac_suffix()).to_owned(),
                    })
                }
                let device2: DeviceModel = DeviceModel {
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
                let mut interfaces2: Vec<Interface> = vec![];
                for i in 0..device2.interface_count {
                    interfaces2.push(Interface {
                        name: format!("{}/{}", device2.interface_prefix, i),
                        num: i,
                        mac_address: format!("{}:{}", KVM_OUI, random_mac_suffix()).to_owned(),
                    })
                }
                let device3: DeviceModel = DeviceModel {
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
                let mut interfaces3: Vec<Interface> = vec![];
                for i in 0..device3.interface_count {
                    interfaces3.push(Interface {
                        name: format!("{}/{}", device3.interface_prefix, i),
                        num: i,
                        mac_address: format!("{}:{}", KVM_OUI, random_mac_suffix()).to_owned(),
                    })
                }

                let template1 = DomainTemplate {
                    name: "iosv1".to_owned(),
                    cpu_count: device1.cpu_count,
                    cpu_architecture: device1.cpu_architecture,
                    machine_type: device1.machine_type,
                    memory: device1.memory,
                    qemu_bin: QEMU_BIN.to_owned(),
                    boot_disk: "/tmp/vios-adventerprisek9-m.SPA.159-3.M6/virtioa1.qcow2".to_owned(),
                    interfaces: interfaces1,
                    interface_type: device1.interface_type,
                };
                let template2 = DomainTemplate {
                    name: "iosv2".to_owned(),
                    cpu_count: device2.cpu_count,
                    cpu_architecture: device2.cpu_architecture,
                    machine_type: device2.machine_type,
                    memory: device2.memory,
                    qemu_bin: QEMU_BIN.to_owned(),
                    boot_disk: "/tmp/vios-adventerprisek9-m.SPA.159-3.M6/virtioa2.qcow2".to_owned(),
                    interfaces: interfaces2,
                    interface_type: device2.interface_type,
                };
                let template3 = DomainTemplate {
                    name: "iosv3".to_owned(),
                    cpu_count: device3.cpu_count,
                    cpu_architecture: device3.cpu_architecture,
                    machine_type: device3.machine_type,
                    memory: device3.memory,
                    qemu_bin: QEMU_BIN.to_owned(),
                    boot_disk: "/tmp/vios-adventerprisek9-m.SPA.159-3.M6/virtioa3.qcow2".to_owned(),
                    interfaces: interfaces3,
                    interface_type: device3.interface_type,
                };

                // Render the XML document
                let rendered_xml1 = template1.render().unwrap();
                let rendered_xml2 = template2.render().unwrap();
                let rendered_xml3 = template3.render().unwrap();

                let xml_configs = vec![rendered_xml1, rendered_xml2, rendered_xml3];
                // println!("{}", rendered_xml);

                let qemu = Qemu::default();
                let qemu_conn = qemu.connect()?;

                /*
                // Define the domain (VM) using the XML data
                let domain = Domain::define_xml(&qemu_conn, &rendered_xml)
                    .expect("Failed to define the domain");

                println!("Domain defined: {:?}", domain.get_name().unwrap());

                // Optionally start the VM
                domain.create().expect("Failed to start the domain");
                println!("Domain started: {:?}", domain.get_name().unwrap());
                */

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
        }
    }
}

fn create_vm(conn: &Connect, xml: &str) -> Result<Domain> {
    let domain = Domain::create_xml(conn, xml, 0)?;
    println!("Domain started: {:?}", domain.get_name().unwrap());
    Ok(domain)
}
