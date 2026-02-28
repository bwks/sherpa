use anyhow::Result;
use clap::{Parser, Subcommand};

use super::doctor::doctor;
use super::image::{ImageCommands, image_commands};
use super::init::init;
use super::user::{UserCommands, user_commands};
use shared::konst::{SHERPA_BASE_DIR, SHERPA_CONFIG_DIR, SHERPA_CONFIG_FILE};
use shared::util::{get_server_url, load_config};

#[derive(Debug, Parser)]
#[command(name = "sherpactl")]
#[command(bin_name = "sherpactl")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Sherpa Control - Administrative Tool", long_about = None)]
pub struct Cli {
    /// Remote server URL for WebSocket RPC (e.g., ws://localhost:3030/ws)
    #[arg(long, global = true, env = "SHERPA_SERVER_URL")]
    server_url: Option<String>,

    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Output format (text or json)
    #[arg(long, global = true, default_value = "text")]
    output: OutputFormat,

    #[clap(subcommand)]
    commands: Commands,
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Initialise the Sherpa server environment
    Init {
        /// Overwrite existing config and keys
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        force: bool,
    },
    /// User management commands
    User {
        #[command(subcommand)]
        commands: UserCommands,
    },
    /// Image management commands
    Image {
        #[command(subcommand)]
        commands: ImageCommands,
    },
    /// Fix up server environment
    Doctor {
        /// Set base box permissions to read-only
        #[arg(long, action = clap::ArgAction::SetTrue)]
        boxes: bool,
    },
}

impl Cli {
    pub async fn run() -> Result<()> {
        let cli = Cli::parse();

        // Setup logging based on verbose flag
        if cli.verbose {
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::DEBUG)
                .init();
        } else {
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::WARN)
                .init();
        }

        // Resolve server URL (CLI > env > config > default)
        let config_dir = format!("{SHERPA_BASE_DIR}/{SHERPA_CONFIG_DIR}");
        let config_file_path = format!("{config_dir}/{SHERPA_CONFIG_FILE}");
        let config = load_config(&config_file_path).ok();

        let server_url = cli
            .server_url
            .or_else(get_server_url)
            .or_else(|| {
                config
                    .as_ref()
                    .and_then(|c| c.server_connection.url.clone())
            })
            .unwrap_or_else(|| {
                config
                    .as_ref()
                    .map(|c| format!("ws://{}:{}/ws", c.server_ipv4, c.server_port))
                    .unwrap_or_else(|| "ws://localhost:3030/ws".to_string())
            });

        match &cli.commands {
            Commands::Init { force } => {
                init(*force).await?;
            }
            Commands::User { commands } => {
                user_commands(commands, &server_url, &cli.output).await?;
            }
            Commands::Image { commands } => {
                image_commands(commands, &server_url, &cli.output).await?;
            }
            Commands::Doctor { boxes } => {
                doctor(*boxes)?;
            }
        }

        Ok(())
    }
}
