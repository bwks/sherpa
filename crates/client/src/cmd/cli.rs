use anyhow::Result;

use clap::{Parser, Subcommand};
// use container::docker_connection;

use super::clean::clean;
use super::console::console;
use super::container::{ContainerCommands, parse_container_commands};
use super::destroy::destroy;
use super::doctor::doctor;
use super::down::down;
use super::image::{ImageCommands, parse_image_commands};
use super::import::import;
use super::init::init;
use super::inspect::inspect;
use super::resume::resume;
use super::ssh::ssh;
use super::up::up;

use data::{DeviceModels, Sherpa};
use konst::{
    SHERPA_BINS_DIR, SHERPA_CONFIG_DIR, SHERPA_CONFIG_FILE, SHERPA_CONTAINERS_DIR,
    SHERPA_IMAGES_DIR, SHERPA_MANIFEST_FILE,
};
use libvirt::Qemu;
use topology::Manifest;
use util::{expand_path, get_id, load_config};

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
    Up {
        /// Name of the config file
        #[arg(default_value = SHERPA_CONFIG_FILE)]
        config_file: String,
    },
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

    /// Import a disk image
    Import {
        /// Source path of the disk image
        #[arg(short, long)]
        src: String,
        /// Version of the device model
        #[arg(short, long)]
        version: String,
        /// Model of Device
        #[arg(short, long, value_enum)]
        model: DeviceModels,
        /// Import the disk image as the latest version
        #[arg(long, action = clap::ArgAction::SetTrue)]
        latest: bool,
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
    /// Image management commands
    Image {
        #[command(subcommand)]
        commands: ImageCommands,
    },
}
impl Default for Commands {
    fn default() -> Self {
        Commands::Init {
            config_file: SHERPA_CONFIG_FILE.to_owned(),
            manifest_file: SHERPA_MANIFEST_FILE.to_owned(),
            force: false,
        }
    }
}

impl Cli {
    pub async fn run() -> Result<()> {
        let cli = Cli::parse();
        let qemu = Qemu::default();
        // let docker = docker_connection()?;
        let sherpa = Sherpa {
            config_dir: expand_path(SHERPA_CONFIG_DIR),
            boxes_dir: expand_path(&format!("{SHERPA_CONFIG_DIR}/{SHERPA_IMAGES_DIR}")),
            config_path: expand_path(&format!("{SHERPA_CONFIG_DIR}/{SHERPA_CONFIG_FILE}")),
            containers_dir: expand_path(&format!("{SHERPA_CONFIG_DIR}/{SHERPA_CONTAINERS_DIR}")),
            bins_dir: expand_path(&format!("{SHERPA_CONFIG_DIR}/{SHERPA_BINS_DIR}")),
        };
        match &cli.commands {
            Commands::Init {
                config_file,
                manifest_file,
                force,
            } => {
                init(&sherpa, &qemu, config_file, manifest_file, *force).await?;
            }

            Commands::Up { config_file } => {
                let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
                let lab_id = get_id(&manifest.name)?;
                let lab_name = manifest.name.clone();

                up(&sherpa, config_file, &qemu, &lab_name, &lab_id, &manifest).await?;
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
                let config = load_config(&sherpa.config_path)?;
                inspect(&qemu, &lab_name, &lab_id, &config, &manifest.nodes).await?;
            }
            Commands::Import {
                src,
                version,
                model,
                latest,
            } => {
                import(src, version, model, *latest, &sherpa.boxes_dir)?;
            }
            Commands::Doctor { boxes } => {
                doctor(*boxes, &sherpa.boxes_dir)?;
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
                ssh(name).await?;
            }
            Commands::Container { commands } => {
                parse_container_commands(commands, &sherpa).await?;
            }
            Commands::Image { commands } => {
                parse_image_commands(commands, &sherpa)?;
            }
        }
        Ok(())
    }
}
