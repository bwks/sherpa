use clap::{Parser, Subcommand};

#[derive(Default, Parser)]
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
        #[arg(default_value = "sherpa.toml")]
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
    pub fn start() -> Cli {
        let cli = Cli::parse();

        match &cli.commands {
            Commands::Init { config_file } => {
                println!("Initializing with config file: {config_file}");
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

        cli
    }
}

impl Default for Commands {
    fn default() -> Self {
        Commands::Init {
            config_file: "sherpa.toml".to_owned(),
        }
    }
}
