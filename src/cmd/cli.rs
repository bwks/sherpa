use std::fs;

use anyhow::Result;
use askama::Template;
use clap::{Parser, Subcommand};

use crate::core::konst::CONFIG_FILENAME;
use crate::core::Config;
use crate::libvirt::DomainTemplate;
use crate::libvirt::Qemu;
use crate::topology::Manifest;
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

                let template = DomainTemplate {
                    name: "iosv",
                    cpus: 1,
                    memory: 1024,
                };

                // Render the XML document
                let rendered_xml = template.render().unwrap();
                println!("{}", rendered_xml);
                /*
                    let qemu = Qemu::default();
                    let qemu_conn = qemu.connect()?;
                    // Load the XML configuration from a file
                    let xml_path = "iosv_test.xml"; // Replace with the actual path to your XML file
                    let xml_data = fs::read_to_string(xml_path)
                        .expect("Failed to read the XML configuration file");

                    // Define the domain (VM) using the XML data
                    let domain =
                        Domain::define_xml(&qemu_conn, &xml_data).expect("Failed to define the domain");

                    println!("Domain defined: {:?}", domain.get_name().unwrap());

                    // Optionally start the VM
                    domain.create().expect("Failed to start the domain");
                    println!("Domain started: {:?}", domain.get_name().unwrap());
                */
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
