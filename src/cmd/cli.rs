use clap::{Parser, Subcommand};

use crate::core::konst::CONFIG_FILENAME;
use crate::core::Config;

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
}

impl Cli {
    pub fn start() -> Config {
        let cli = Cli::parse();
        let mut config = Config::default();

        match &cli.commands {
            Commands::Init { config_file } => {
                config.name = config_file.to_owned();
                println!("Initializing with config file: {:#?}", config.name);
            }
            Commands::Up => {
                println!("Building environment");
            }
            Commands::Down => {
                println!("Stopping environment");
            }
            Commands::Destroy => {
                println!("Destroying environment");
            }
        }

        config
    }
}

impl Default for Commands {
    fn default() -> Self {
        Commands::Init {
            config_file: CONFIG_FILENAME.to_owned(),
        }
    }
}
