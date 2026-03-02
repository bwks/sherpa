use anyhow::Result;
use clap::{Parser, Subcommand};

use super::doctor::doctor;
use super::image::{ImageCommands, image_commands};
use super::init::init;
use super::user::{UserCommands, user_commands};
use std::path::Path;

use shared::konst::{SHERPA_CONFIG_FILE_PATH, SHERPA_ENV_FILE_PATH};
use shared::util::{build_websocket_url, get_server_url, load_config, read_env_file_value};

#[derive(Debug, Parser)]
#[command(name = "sherpactl")]
#[command(bin_name = "sherpactl")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Sherpa Control - Administrative Tool", long_about = None)]
pub struct Cli {
    /// Remote server URL for WebSocket RPC (e.g., ws://localhost:3030/ws)
    #[arg(long, global = true, env = "SHERPA_SERVER_URL")]
    server_url: Option<String>,

    /// Skip TLS certificate validation (INSECURE)
    #[arg(long, global = true)]
    insecure: bool,

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

        /// SurrealDB password (also reads from SHERPA_DB_PASSWORD env var or /opt/sherpa/env/sherpa.env)
        #[arg(long = "db-pass", env = "SHERPA_DB_PASSWORD")]
        db_password: Option<String>,

        /// Server listen IP address (also reads from SHERPA_SERVER_IP env var or /opt/sherpa/env/sherpa.env)
        #[arg(long = "server-ip", env = "SHERPA_SERVER_IP")]
        server_ip: Option<String>,
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
        let config = load_config(SHERPA_CONFIG_FILE_PATH).ok();

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
                    .map(build_websocket_url)
                    .unwrap_or_else(|| "ws://localhost:3030/ws".to_string())
            });

        // Build ServerConnection from config, with CLI overrides
        let mut server_connection = config
            .as_ref()
            .map(|c| c.server_connection.clone())
            .unwrap_or_default();

        // Override insecure if CLI flag is set
        if cli.insecure {
            server_connection.insecure = true;
        }

        match &cli.commands {
            Commands::Init {
                force,
                db_password,
                server_ip,
            } => {
                let env_file = Path::new(SHERPA_ENV_FILE_PATH);

                let password = match db_password {
                    Some(p) => p.clone(),
                    None => {
                        read_env_file_value(env_file, "SHERPA_DB_PASSWORD").ok_or_else(|| {
                            anyhow::anyhow!(
                                "Database password not provided. Supply it via:\n  \
                                 1. --db-pass flag\n  \
                                 2. SHERPA_DB_PASSWORD environment variable\n  \
                                 3. SHERPA_DB_PASSWORD entry in {}",
                                env_file.display()
                            )
                        })?
                    }
                };

                let ip = match server_ip {
                    Some(ip) => ip.clone(),
                    None => read_env_file_value(env_file, "SHERPA_SERVER_IP")
                        .unwrap_or_else(|| "0.0.0.0".to_string()),
                };

                init(*force, &password, &ip).await?;
            }
            Commands::User { commands } => {
                user_commands(commands, &server_url, &server_connection, &cli.output).await?;
            }
            Commands::Image { commands } => {
                image_commands(commands, &server_url, &server_connection, &cli.output).await?;
            }
            Commands::Doctor { boxes } => {
                doctor(*boxes)?;
            }
        }

        Ok(())
    }
}
