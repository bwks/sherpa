use anyhow::Result;
use clap::{Parser, Subcommand};

use super::cert::{cert_delete, cert_list, cert_show, cert_trust};
use super::clean::clean;
use super::console::console;
use super::destroy::destroy;
use super::down::down;
use super::image::{ImageCommands, parse_image_commands};
use super::init::init;
use super::inspect::inspect;
use super::login::{login, logout, whoami};
use super::new::new;
use super::resume::resume;
use super::ssh::ssh;
use super::up::up;
use super::validate::validate_manifest;
use super::virtual_machine::VirtualMachineCommands;

use shared::data::ClientConfig;
use shared::data::Sherpa;
use shared::konst::SHERPA_MANIFEST_FILE;
use shared::util::{build_client_websocket_url, get_id, get_server_url, load_client_config};
use topology::Manifest;

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
    Up {
        /// Path to manifest file (defaults to manifest.toml)
        #[arg(long, default_value = SHERPA_MANIFEST_FILE)]
        manifest: String,
    },

    /// Stop environment
    Down,
    /// Resume environment
    Resume,
    /// Destroy environment
    Destroy,
    /// Inspect environment
    #[default]
    Inspect,

    /// Validate configurations
    Validate {
        /// Path to manifest file to validate (defaults to manifest.toml)
        #[arg(long)]
        manifest: Option<String>,
    },

    /// Force clean all resources for a lab (admin-only)
    Clean {
        /// Lab ID to clean (if omitted, derived from manifest)
        lab_id: Option<String>,
    },

    /// Connect to a device via serial console over Telnet
    Console { name: String },

    /// SSH to a device.
    Ssh { name: String },

    /// Virtual machine management commands
    Vm {
        #[command(subcommand)]
        commands: VirtualMachineCommands,
    },
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

            Commands::Up { manifest } => {
                // Load manifest to get lab name
                let manifest_obj = Manifest::load_file(manifest)?;
                let lab_id = get_id(&manifest_obj.name)?;
                let lab_name = manifest_obj.name.clone();
                let mut config = load_client_config_or_default(&sherpa.config_file_path);

                // Apply --insecure flag if set
                if cli.insecure {
                    config.server_connection.insecure = true;
                    eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
                }

                let server_url = resolve_server_url(cli.server_url, &config);
                up(&lab_name, &lab_id, manifest, &server_url, &config).await?;
            }
            Commands::Down => {
                let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                let lab_id = get_id(&manifest.name)?;
                let lab_name = manifest.name.clone();
                let mut config = load_client_config_or_default(&sherpa.config_file_path);

                if cli.insecure {
                    config.server_connection.insecure = true;
                    eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
                }

                let server_url = resolve_server_url(cli.server_url, &config);
                down(&lab_name, &lab_id, &server_url, &config).await?;
            }
            Commands::Resume => {
                let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                let lab_id = get_id(&manifest.name)?;
                let lab_name = manifest.name.clone();
                let mut config = load_client_config_or_default(&sherpa.config_file_path);

                if cli.insecure {
                    config.server_connection.insecure = true;
                    eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
                }

                let server_url = resolve_server_url(cli.server_url, &config);
                resume(&lab_name, &lab_id, &server_url, &config).await?;
            }
            Commands::Destroy => {
                let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                let lab_id = get_id(&manifest.name)?;
                let lab_name = manifest.name.clone();
                let mut config = load_client_config_or_default(&sherpa.config_file_path);

                if cli.insecure {
                    config.server_connection.insecure = true;
                    eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
                }

                let server_url = resolve_server_url(cli.server_url, &config);
                destroy(&lab_name, &lab_id, &server_url, &config).await?;
            }
            Commands::Inspect => {
                let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                let lab_id = get_id(&manifest.name)?;
                let lab_name = manifest.name.clone();
                let mut config = load_client_config_or_default(&sherpa.config_file_path);

                if cli.insecure {
                    config.server_connection.insecure = true;
                    eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
                }

                let server_url = resolve_server_url(cli.server_url, &config);
                inspect(&lab_name, &lab_id, &server_url, &config).await?;
            }
            Commands::Validate { manifest } => {
                // Default to manifest.toml if no specific flag provided
                let manifest_path = manifest.as_deref().unwrap_or(SHERPA_MANIFEST_FILE);
                validate_manifest(manifest_path)?;
            }
            Commands::Clean { lab_id } => {
                let lab_id = match lab_id {
                    Some(id) => id.clone(),
                    None => {
                        let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                        get_id(&manifest.name)?
                    }
                };
                let mut config = load_client_config_or_default(&sherpa.config_file_path);

                if cli.insecure {
                    config.server_connection.insecure = true;
                    eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
                }

                let server_url = resolve_server_url(cli.server_url, &config);
                clean(&lab_id, &server_url, &config).await?;
            }
            Commands::Console { name } => {
                let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                console(name, &manifest)?;
            }
            Commands::Ssh { name } => {
                ssh(name).await?;
            }
            Commands::Vm { commands } => {
                // parse_vm_commands(commands, &sherpa).await?;
                let _cmds = commands;
                println!("NOT IMPLEMENTED");
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
        }
        Ok(())
    }
}
