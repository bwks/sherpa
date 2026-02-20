use anyhow::Result;

use clap::{Parser, Subcommand};

use super::cert::{cert_delete, cert_list, cert_show, cert_trust};
use super::clean::clean;
use super::console::console;
use super::container::{ContainerCommands, parse_container_commands};
use super::destroy::destroy;
use super::doctor::doctor;
use super::down::down;
use super::image::{ImageCommands, parse_image_commands};
use super::init::init;
use super::inspect::inspect;
use super::login::{login, logout, whoami};
use super::resume::resume;
use super::ssh::ssh;
use super::unikernel::UnikernelCommands;
use super::up::up;
use super::validate::validate_manifest;
use super::virtual_machine::VirtualMachineCommands;

use libvirt::Qemu;
use shared::data::Sherpa;
use shared::konst::{
    SHERPA_BASE_DIR, SHERPA_BINS_DIR, SHERPA_CONFIG_DIR, SHERPA_CONFIG_FILE, SHERPA_CONTAINERS_DIR,
    SHERPA_IMAGES_DIR, SHERPA_MANIFEST_FILE, SHERPA_SSH_DIR,
};
use shared::util::{build_websocket_url, get_id, get_server_url, load_config};
use topology::Manifest;

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

#[derive(Debug, Subcommand)]
enum Commands {
    /// Login to Sherpa server
    Login,
    /// Logout from Sherpa server
    Logout,
    /// Show current authentication status
    Whoami,

    /// Initialise a Sherpa environment
    Init {
        /// Name of the config file
        #[arg(default_value = SHERPA_CONFIG_FILE)]
        config_file: String,

        /// Name of the manifest file
        #[arg(default_value = SHERPA_MANIFEST_FILE)]
        manifest_file: String,

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
    Inspect,

    /// Validate configurations
    Validate {
        /// Path to manifest file to validate (defaults to manifest.toml)
        #[arg(long)]
        manifest: Option<String>,
    },

    /// Fix up environment
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
    Container {
        #[command(subcommand)]
        commands: ContainerCommands,
    },
    /// Container management commands
    Vm {
        #[command(subcommand)]
        commands: VirtualMachineCommands,
    },
    /// Container management commands
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

impl Default for Commands {
    fn default() -> Self {
        let config_dir = format!("{SHERPA_BASE_DIR}/{SHERPA_CONFIG_DIR}");
        Commands::Init {
            config_file: format!("{config_dir}/{SHERPA_CONFIG_FILE}"),
            manifest_file: SHERPA_MANIFEST_FILE.to_string(),
            force: false,
        }
    }
}

impl Cli {
    pub async fn run() -> Result<()> {
        let cli = Cli::parse();
        let qemu = Qemu::default();
        let config_dir = format!("{SHERPA_BASE_DIR}/{SHERPA_CONFIG_DIR}");
        let sherpa = Sherpa {
            base_dir: SHERPA_BASE_DIR.to_string(),
            config_file_path: format!("{config_dir}/{SHERPA_CONFIG_FILE}"),
            ssh_dir: format!("{SHERPA_BASE_DIR}/{SHERPA_SSH_DIR}"),
            images_dir: format!("{SHERPA_BASE_DIR}/{SHERPA_IMAGES_DIR}"),
            containers_dir: format!("{SHERPA_BASE_DIR}/{SHERPA_CONTAINERS_DIR}"),
            bins_dir: format!("{SHERPA_BASE_DIR}/{SHERPA_BINS_DIR}"),
            config_dir,
        };
        match &cli.commands {
            Commands::Login => {
                let config = load_config(&sherpa.config_file_path).ok();

                // Resolve server URL (CLI > env > config > explicit URL > build from config)
                let server_url = cli
                    .server_url
                    .or_else(get_server_url)
                    .or_else(|| config.as_ref().map(build_websocket_url))
                    .unwrap_or_else(|| "ws://localhost:3030/ws".to_string());

                login(&server_url, cli.insecure).await?;
            }
            Commands::Logout => {
                logout()?;
            }
            Commands::Whoami => {
                let config = load_config(&sherpa.config_file_path).ok();

                // Resolve server URL (CLI > env > config > explicit URL > build from config)
                let server_url = cli
                    .server_url
                    .or_else(get_server_url)
                    .or_else(|| config.as_ref().map(build_websocket_url))
                    .unwrap_or_else(|| "ws://localhost:3030/ws".to_string());

                whoami(&server_url, cli.insecure).await?;
            }
            Commands::Init {
                config_file,
                manifest_file,
                force,
            } => {
                init(&sherpa, &qemu, config_file, manifest_file, *force).await?;
            }

            Commands::Up { manifest } => {
                // Load manifest to get lab name
                let manifest_obj = Manifest::load_file(manifest)?;
                let lab_id = get_id(&manifest_obj.name)?;
                let lab_name = manifest_obj.name.clone();
                let mut config = load_config(&sherpa.config_file_path)?;

                // Apply --insecure flag if set
                if cli.insecure {
                    config.server_connection.insecure = true;
                    eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
                }

                // Resolve server URL (CLI > env > config > build from config)
                let server_url = cli
                    .server_url
                    .or_else(get_server_url)
                    .unwrap_or_else(|| build_websocket_url(&config));

                up(&lab_name, &lab_id, manifest, &server_url, &config).await?;
            }
            Commands::Down => {
                let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                let lab_id = get_id(&manifest.name)?;
                down(&qemu, &lab_id)?;
            }
            Commands::Resume => {
                let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                let lab_id = get_id(&manifest.name)?;
                resume(&qemu, &lab_id)?;
            }
            Commands::Destroy => {
                let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                let lab_id = get_id(&manifest.name)?;
                let lab_name = manifest.name.clone();
                let mut config = load_config(&sherpa.config_file_path)?;

                // Apply --insecure flag if set
                if cli.insecure {
                    config.server_connection.insecure = true;
                    eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
                }

                // Resolve server URL (CLI > env > config > build from config)
                let server_url = cli
                    .server_url
                    .or_else(get_server_url)
                    .unwrap_or_else(|| build_websocket_url(&config));

                destroy(&lab_name, &lab_id, &server_url, &config).await?;
            }
            Commands::Inspect => {
                let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                let lab_id = get_id(&manifest.name)?;
                let lab_name = manifest.name.clone();
                let mut config = load_config(&sherpa.config_file_path)?;

                // Apply --insecure flag if set
                if cli.insecure {
                    config.server_connection.insecure = true;
                    eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
                }

                // Resolve server URL (CLI > env > config > build from config)
                let server_url = cli
                    .server_url
                    .or_else(get_server_url)
                    .unwrap_or_else(|| build_websocket_url(&config));

                inspect(&lab_name, &lab_id, &server_url, &config).await?;
            }
            Commands::Validate { manifest } => {
                // Default to manifest.toml if no specific flag provided
                let manifest_path = manifest.as_deref().unwrap_or(SHERPA_MANIFEST_FILE);
                validate_manifest(manifest_path)?;
            }
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
                let mut config = load_config(&sherpa.config_file_path)?;

                if cli.insecure {
                    config.server_connection.insecure = true;
                    eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
                }

                let server_url = cli
                    .server_url
                    .or_else(get_server_url)
                    .unwrap_or_else(|| build_websocket_url(&config));

                clean(&lab_id, &server_url, &config).await?;
            }
            Commands::Console { name } => {
                let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                console(name, &manifest)?;
            }
            Commands::Ssh { name } => {
                ssh(name).await?;
            }
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
                let mut config = load_config(&sherpa.config_file_path)?;

                if cli.insecure {
                    config.server_connection.insecure = true;
                    eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
                }

                let server_url = cli
                    .server_url
                    .or_else(get_server_url)
                    .unwrap_or_else(|| build_websocket_url(&config));

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
