use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use super::cert::{cert_delete, cert_list, cert_show, cert_trust};
use super::console::console;
use super::destroy::destroy;
use super::down::down;
use super::image::{ImageCommands, parse_image_commands};
use super::init::init;
use super::inspect::inspect;
use super::login::{login, logout, whoami};
use super::new::new;
use super::redeploy::redeploy;
use super::resume::resume;
use super::server::{OutputFormat, ServerCommands, run_server};
use super::ssh::ssh;
use super::up::up;
use super::validate::validate_manifest;

use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

use shared::data::{ClientConfig, InspectResponse, LabIdentity, LabInfo, Sherpa};
use shared::konst::{LAB_FILE_NAME, SHERPA_MANIFEST_FILE};
use shared::util::{
    build_client_websocket_url, file_exists, get_cwd, get_id, get_server_url, load_client_config,
    load_file,
};
use topology::Manifest;

/// Resolve lab identity (name + id) from the best available source.
///
/// Priority:
/// 1. Try `lab-info.toml` in the current directory (written by `sherpa up`)
/// 2. Fall back to `manifest.toml` in the current directory
fn resolve_lab_identity() -> anyhow::Result<LabIdentity> {
    // Try lab-info.toml first (already has id and name)
    if let Ok(cwd) = get_cwd() {
        let lab_info_path = format!("{}/{}", cwd, LAB_FILE_NAME);
        if file_exists(&lab_info_path)
            && let Ok(content) = load_file(&lab_info_path)
            && let Ok(lab_info) = content.parse::<LabInfo>()
        {
            return Ok(LabIdentity {
                name: lab_info.name,
                id: lab_info.id,
            });
        }
    }

    // Fall back to manifest.toml
    let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
    let id = get_id(&manifest.name)?;
    Ok(LabIdentity {
        name: manifest.name,
        id,
    })
}

/// Load client config from file, falling back to defaults if the file doesn't exist.
fn load_client_config_or_default(path: &str) -> ClientConfig {
    load_client_config(path).unwrap_or_else(|_| ClientConfig::default())
}

/// Resolve the server URL from CLI args, env, or config.
fn resolve_server_url(cli_url: Option<String>, config: &ClientConfig) -> String {
    cli_url
        .or_else(get_server_url)
        .unwrap_or_else(|| build_client_websocket_url(config))
}

/// Resolve lab info: try lab-info.toml first, fall back to inspect RPC.
async fn resolve_lab_info(
    lab_id: &str,
    server_url: &str,
    config: &ClientConfig,
) -> Result<LabInfo> {
    // Fast path: try local lab-info.toml
    if let Ok(cwd) = get_cwd() {
        let lab_info_path = format!("{}/{}", cwd, LAB_FILE_NAME);
        if file_exists(&lab_info_path)
            && let Ok(content) = load_file(&lab_info_path)
            && let Ok(lab_info) = LabInfo::from_str(&content)
        {
            return Ok(lab_info);
        }
    }

    // Slow path: fetch from server via inspect RPC
    let token = load_token()
        .context("lab-info.toml not found and no auth token available. Run: sherpa login")?;

    let timeout = Duration::from_secs(config.server_connection.timeout_secs);
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        timeout,
        config.server_connection.clone(),
    );

    let mut rpc_client = ws_client
        .connect()
        .await
        .context("lab-info.toml not found and failed to connect to server")?;

    let request = RpcRequest::new(
        "inspect",
        serde_json::json!({
            "lab_id": lab_id,
            "token": token,
        }),
    );

    let response = rpc_client
        .call(request)
        .await
        .context("RPC inspect call failed")?;
    rpc_client.close().await.ok();

    if let Some(error) = response.error {
        bail!("Failed to fetch lab info from server: {}", error.message);
    }

    let result = response.result.context("No result in inspect response")?;
    let inspect_data: InspectResponse =
        serde_json::from_value(result).context("Failed to parse inspect response")?;

    Ok(inspect_data.lab_info)
}

/// Resolve the Sherpa client base directory (~/.sherpa)
fn sherpa_client_base_dir() -> String {
    dirs::home_dir()
        .map(|h| h.join(".sherpa").to_string_lossy().to_string())
        .unwrap_or_else(|| ".sherpa".to_string())
}

#[derive(Default, Debug, Parser)]
#[command(name = "sherpa")]
#[command(bin_name = "sherpa")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Sherpa - Network Lab Management", long_about = None)]
pub struct Cli {
    /// Remote server URL for WebSocket RPC (e.g., ws://localhost:3030/ws)
    #[arg(long, global = true, env = "SHERPA_SERVER_URL")]
    server_url: Option<String>,

    /// Skip TLS certificate validation (insecure - for development only)
    #[arg(long, global = true)]
    insecure: bool,

    #[clap(subcommand)]
    commands: Commands,
}

#[derive(Default, Debug, Subcommand)]
enum Commands {
    /// Login to Sherpa server
    Login,
    /// Logout from Sherpa server
    Logout,
    /// Show current authentication status
    Whoami,

    /// Create a new example manifest.toml in the current directory
    New {
        /// Overwrite manifest file if one exists
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        force: bool,
    },
    /// Initialise a Sherpa client environment
    Init {
        /// Overwrite config file if one exists
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        force: bool,
    },
    /// Build environment
    Up,

    /// Stop environment
    Down {
        /// Target a specific node (omit for all nodes)
        #[arg(short, long)]
        node: Option<String>,
    },
    /// Resume environment
    Resume {
        /// Target a specific node (omit for all nodes)
        #[arg(short, long)]
        node: Option<String>,
    },
    /// Redeploy a single node (destroy and recreate with fresh ZTP)
    Redeploy {
        /// Target node to redeploy
        #[arg(short, long)]
        node: String,
    },
    /// Destroy environment
    Destroy {
        /// Skip confirmation prompt
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        yes: bool,
    },
    /// Inspect environment
    #[default]
    Inspect,

    /// Validate configurations
    Validate,

    /// Connect to a device via serial console over Telnet
    Console { name: String },

    /// SSH to a device.
    Ssh { name: String },

    /// Image management commands
    Image {
        #[command(subcommand)]
        commands: ImageCommands,
    },

    /// Certificate management commands (TOFU system)
    Cert {
        #[command(subcommand)]
        commands: CertCommands,
    },

    /// Server administration commands
    Server {
        /// Enable verbose logging
        #[arg(short, long)]
        verbose: bool,

        /// Output format (text or json)
        #[arg(long, default_value = "text")]
        output: OutputFormat,

        #[command(subcommand)]
        commands: ServerCommands,
    },
}

#[derive(Debug, Subcommand)]
enum CertCommands {
    /// List all trusted certificates
    List,
    /// Show detailed certificate information
    Show {
        /// Server URL (e.g., wss://10.100.58.10:3030/ws)
        server_url: String,
    },
    /// Trust a certificate without connecting
    Trust {
        /// Server URL (e.g., wss://10.100.58.10:3030/ws)
        server_url: String,
    },
    /// Delete a trusted certificate
    Delete {
        /// Server URL (e.g., wss://10.100.58.10:3030/ws)
        server_url: String,
    },
}

impl Cli {
    pub async fn run() -> Result<()> {
        let cli = Cli::parse();
        let sherpa = Sherpa::from_base_dir(sherpa_client_base_dir());
        match &cli.commands {
            Commands::Login => {
                let config = load_client_config_or_default(&sherpa.config_file_path);

                let server_url = cli
                    .server_url
                    .or_else(get_server_url)
                    .unwrap_or_else(|| build_client_websocket_url(&config));

                login(&server_url, cli.insecure, &config).await?;
            }
            Commands::Logout => {
                logout()?;
            }
            Commands::Whoami => {
                let config = load_client_config_or_default(&sherpa.config_file_path);

                let server_url = cli
                    .server_url
                    .or_else(get_server_url)
                    .unwrap_or_else(|| build_client_websocket_url(&config));

                whoami(&server_url, cli.insecure, &config).await?;
            }
            Commands::New { force } => {
                new(*force)?;
            }
            Commands::Init { force } => {
                init(&sherpa, *force)?;
            }

            Commands::Up => {
                let manifest_obj = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                let lab_id = get_id(&manifest_obj.name)?;
                let lab_name = manifest_obj.name.clone();
                let mut config = load_client_config_or_default(&sherpa.config_file_path);

                if cli.insecure {
                    config.server_connection.insecure = true;
                    eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
                }

                let server_url = resolve_server_url(cli.server_url, &config);
                up(
                    &lab_name,
                    &lab_id,
                    SHERPA_MANIFEST_FILE,
                    &server_url,
                    &config,
                )
                .await?;
            }
            Commands::Down { node } => {
                let lab = resolve_lab_identity()?;
                let mut config = load_client_config_or_default(&sherpa.config_file_path);

                if cli.insecure {
                    config.server_connection.insecure = true;
                    eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
                }

                let server_url = resolve_server_url(cli.server_url, &config);
                down(&lab.name, &lab.id, node.as_deref(), &server_url, &config).await?;
            }
            Commands::Resume { node } => {
                let lab = resolve_lab_identity()?;
                let mut config = load_client_config_or_default(&sherpa.config_file_path);

                if cli.insecure {
                    config.server_connection.insecure = true;
                    eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
                }

                let server_url = resolve_server_url(cli.server_url, &config);
                resume(&lab.name, &lab.id, node.as_deref(), &server_url, &config).await?;
            }
            Commands::Redeploy { node } => {
                let manifest_obj = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                let lab_id = get_id(&manifest_obj.name)?;
                let lab_name = manifest_obj.name.clone();
                let mut config = load_client_config_or_default(&sherpa.config_file_path);

                if cli.insecure {
                    config.server_connection.insecure = true;
                    eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
                }

                let server_url = resolve_server_url(cli.server_url, &config);
                redeploy(
                    &lab_name,
                    &lab_id,
                    node,
                    SHERPA_MANIFEST_FILE,
                    &server_url,
                    &config,
                )
                .await?;
            }
            Commands::Destroy { yes } => {
                let lab = resolve_lab_identity()?;
                let mut config = load_client_config_or_default(&sherpa.config_file_path);

                if cli.insecure {
                    config.server_connection.insecure = true;
                    eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
                }

                let server_url = resolve_server_url(cli.server_url, &config);
                destroy(&lab.name, &lab.id, &server_url, &config, *yes).await?;
            }
            Commands::Inspect => {
                let lab = resolve_lab_identity()?;
                let mut config = load_client_config_or_default(&sherpa.config_file_path);

                if cli.insecure {
                    config.server_connection.insecure = true;
                    eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
                }

                let server_url = resolve_server_url(cli.server_url, &config);
                inspect(&lab.name, &lab.id, &server_url, &config).await?;
            }
            Commands::Validate => {
                validate_manifest(SHERPA_MANIFEST_FILE)?;
            }
            Commands::Console { name } => {
                let manifest_obj = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                let lab_id = get_id(&manifest_obj.name)?;
                let mut config = load_client_config_or_default(&sherpa.config_file_path);

                if cli.insecure {
                    config.server_connection.insecure = true;
                    eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
                }

                let server_url = resolve_server_url(cli.server_url, &config);
                let lab_info = resolve_lab_info(&lab_id, &server_url, &config).await?;
                console(name, &manifest_obj, &lab_info)?;
            }
            Commands::Ssh { name } => {
                let lab = resolve_lab_identity()?;
                ssh(name, &lab.id).await?;
            }
            Commands::Image { commands } => {
                let mut config = load_client_config_or_default(&sherpa.config_file_path);

                if cli.insecure {
                    config.server_connection.insecure = true;
                    eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
                }

                let server_url = resolve_server_url(cli.server_url, &config);
                parse_image_commands(commands, &config, &server_url).await?;
            }
            Commands::Cert { commands } => match commands {
                CertCommands::List => cert_list().await?,
                CertCommands::Show { server_url } => cert_show(server_url).await?,
                CertCommands::Trust { server_url } => cert_trust(server_url).await?,
                CertCommands::Delete { server_url } => cert_delete(server_url).await?,
            },
            Commands::Server {
                verbose,
                output,
                commands,
            } => {
                let mut config = load_client_config_or_default(&sherpa.config_file_path);

                if cli.insecure {
                    config.server_connection.insecure = true;
                }

                let server_url = resolve_server_url(cli.server_url, &config);
                run_server(commands, *verbose, output, &server_url, &config).await?;
            }
        }
        Ok(())
    }
}
