use anyhow::Result;

use clap::{Parser, Subcommand};

use virt::connect::Connect;

use crate::core::konst::CONFIG_FILENAME;
use crate::core::Config;
use crate::topology::Manifest;

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
            }
            Commands::Down => {
                println!("Stopping environment");
            }
            Commands::Destroy => {
                println!("Destroying environment");
            }
            Commands::Inspect => {
                let conn = Connect::open(Some("qemu:///system")).unwrap();
                println!("Connected to hypervisor: {:?}", conn);

                let domains = conn.list_all_domains(0).unwrap();
                for domain in domains {
                    println!("VM Name: {:?}", domain.get_name().unwrap());
                    // println!("VM XML: {:?}", domain.get_xml_desc(0));
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
