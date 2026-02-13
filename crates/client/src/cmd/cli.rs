use anyhow::Result;

use clap::{Parser, Subcommand};

use super::clean::clean;
use super::console::console;
use super::container::{ContainerCommands, parse_container_commands};
use super::destroy::destroy;
use super::doctor::doctor;
use super::down::down;
use super::image::{ImageCommands, parse_image_commands};
use super::init::init;
use super::inspect::inspect;
use super::resume::resume;
use super::ssh::ssh;
use super::unikernel::UnikernelCommands;
use super::up::up;
use super::virtual_machine::VirtualMachineCommands;

use libvirt::Qemu;
use shared::data::Sherpa;
use shared::konst::{
    SHERPA_BASE_DIR, SHERPA_BINS_DIR, SHERPA_CONFIG_DIR, SHERPA_CONFIG_FILE, SHERPA_CONTAINERS_DIR,
    SHERPA_IMAGES_DIR, SHERPA_MANIFEST_FILE, SHERPA_SSH_DIR,
};
use shared::util::{get_id, load_config};
use topology::Manifest;

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
    Up,

    /// Stop environment
    Down,
    /// Resume environment
    Resume,
    /// Destroy environment
    Destroy,
    /// Inspect environment
    Inspect,

    /// Fix up environment
    Doctor {
        /// Set base box permissions to read-only
        #[arg(long, action = clap::ArgAction::SetTrue)]
        boxes: bool,
    },

    /// Clean up environment
    Clean {
        /// Remove all devices, disks and networks
        #[arg(long, action = clap::ArgAction::SetTrue)]
        all: bool,
        /// Remove all disks
        #[arg(long, action = clap::ArgAction::SetTrue)]
        disks: bool,
        /// Remove all networks
        #[arg(long, action = clap::ArgAction::SetTrue)]
        networks: bool,
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
            Commands::Init {
                config_file,
                manifest_file,
                force,
            } => {
                init(&sherpa, &qemu, config_file, manifest_file, *force).await?;
            }

            Commands::Up => {
                let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                let lab_id = get_id(&manifest.name)?;
                let lab_name = manifest.name.clone();

                up(&sherpa, &qemu, &lab_name, &lab_id, &manifest).await?;
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
                destroy(&qemu, &lab_name, &lab_id).await?;
            }
            Commands::Inspect => {
                let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                let lab_id = get_id(&manifest.name)?;
                let lab_name = manifest.name.clone();
                let config = load_config(&sherpa.config_file_path)?;
                inspect(&qemu, &lab_name, &lab_id, &config, &manifest.nodes).await?;
            }
            Commands::Doctor { boxes } => {
                doctor(*boxes, &sherpa.images_dir)?;
            }
            Commands::Clean {
                all,
                disks,
                networks,
            } => {
                let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                let lab_id = get_id(&manifest.name)?;
                clean(&qemu, *all, *disks, *networks, &lab_id)?;
            }
            Commands::Console { name } => {
                let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                console(name, &manifest)?;
            }
            Commands::Ssh { name } => {
                let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                let lab_id = get_id(&manifest.name)?;
                ssh(&lab_id, name).await?;
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
                parse_image_commands(commands, &sherpa).await?;
            }
        }
        Ok(())
    }
}
