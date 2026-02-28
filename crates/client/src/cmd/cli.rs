use anyhow::Result;

use clap::{Parser, Subcommand};

use super::cert::{cert_delete, cert_list, cert_show, cert_trust};
use super::clean::clean;
use super::console::console;
#[cfg(feature = "local")]
use super::container::{ContainerCommands, parse_container_commands};
use super::destroy::destroy;
#[cfg(feature = "local")]
use super::doctor::doctor;
#[cfg(feature = "local")]
use super::down::down;
use super::image::{ImageCommands, parse_image_commands};
use super::init::init;
use super::inspect::inspect;
use super::login::{login, logout, whoami};
#[cfg(feature = "local")]
use super::resume::resume;
use super::ssh::ssh;
use super::unikernel::UnikernelCommands;
use super::up::up;
use super::validate::validate_manifest;
use super::virtual_machine::VirtualMachineCommands;

#[cfg(feature = "local")]
use libvirt::Qemu;
use shared::data::ClientConfig;
use shared::data::Sherpa;
use shared::konst::{
    SHERPA_BINS_DIR, SHERPA_CONFIG_DIR, SHERPA_CONFIG_FILE, SHERPA_CONTAINERS_DIR,
    SHERPA_IMAGES_DIR, SHERPA_MANIFEST_FILE, SHERPA_SSH_DIR,
};
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
    #[cfg(feature = "local")]
    Down,
    /// Resume environment
    #[cfg(feature = "local")]
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

    /// Fix up environment
    #[cfg(feature = "local")]
    Doctor {
        /// Set base box permissions to read-only
        #[arg(long, action = clap::ArgAction::SetTrue)]
        boxes: bool,
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

    /// Container management commands
    #[cfg(feature = "local")]
    Container {
        #[command(subcommand)]
        commands: ContainerCommands,
    },
    /// Virtual machine management commands
    Vm {
        #[command(subcommand)]
        commands: VirtualMachineCommands,
    },
    /// Unikernel management commands
    Unikernel {
        #[command(subcommand)]
        commands: UnikernelCommands,
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
        let base_dir = sherpa_client_base_dir();
        let config_dir = format!("{base_dir}/{SHERPA_CONFIG_DIR}");
        let sherpa = Sherpa {
            base_dir: base_dir.clone(),
            config_file_path: format!("{config_dir}/{SHERPA_CONFIG_FILE}"),
            ssh_dir: format!("{base_dir}/{SHERPA_SSH_DIR}"),
            images_dir: format!("{base_dir}/{SHERPA_IMAGES_DIR}"),
            containers_dir: format!("{base_dir}/{SHERPA_CONTAINERS_DIR}"),
            bins_dir: format!("{base_dir}/{SHERPA_BINS_DIR}"),
            config_dir,
        };
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
            #[cfg(feature = "local")]
            Commands::Down => {
                let qemu = Qemu::default();
                let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                let lab_id = get_id(&manifest.name)?;
                down(&qemu, &lab_id)?;
            }
            #[cfg(feature = "local")]
            Commands::Resume => {
                let qemu = Qemu::default();
                let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                let lab_id = get_id(&manifest.name)?;
                resume(&qemu, &lab_id)?;
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
            #[cfg(feature = "local")]
            Commands::Doctor { boxes } => {
                doctor(*boxes, &sherpa.images_dir)?;
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
            #[cfg(feature = "local")]
            Commands::Container { commands } => {
                parse_container_commands(commands, &sherpa).await?;
            }
            Commands::Vm { commands } => {
                // parse_vm_commands(commands, &sherpa).await?;
                let _cmds = commands;
                println!("NOT IMPLEMENTED");
            }
            Commands::Unikernel { commands } => {
                // parse_unikernel_commands(commands, &sherpa).await?;
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
                parse_image_commands(commands, &sherpa, &config, &server_url).await?;
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
