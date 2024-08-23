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
    #[command(arg_required_else_help = true)]
    Init { config: String },
}

impl Cli {
    pub fn start() -> Cli {
        let cli = Cli::parse();
        cli
    }
}

impl Default for Commands {
    fn default() -> Self {
        Commands::Init {
            config: "sherpa.toml".to_owned(),
        }
    }
}
