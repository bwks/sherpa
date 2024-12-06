use anyhow::Result;

use clap::{Parser, Subcommand};

use crate::cmd::{clean, console, destroy, doctor, down, import, init, inspect, resume, ssh, up};
use crate::core::konst::{SHERPA_CONFIG_FILE, SHERPA_MANIFEST_FILE};
use crate::core::Sherpa;
use crate::data::DeviceModels;
use crate::libvirt::Qemu;
use crate::topology::Manifest;
use crate::util::get_id;

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
    pub fn run() -> Result<()> {
        let cli = Cli::parse();
        let qemu = Qemu::default();
        let sherpa = Sherpa::default();
        let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;

        let lab_id = get_id()?;
        let lab_name = manifest.name.clone();

        match &cli.commands {
            Commands::Init {
                config_file,
                manifest_file,
                force,
            } => {
                init(&sherpa, &qemu, config_file, manifest_file, *force)?;
            }

            Commands::Up { config_file } => {
                up(&sherpa, config_file, &qemu, &lab_name, &lab_id, &manifest)?;
            }
            Commands::Down => {
                down(&qemu, &lab_id)?;
            }
            Commands::Resume => {
                resume(&qemu, &lab_id)?;
            }
            Commands::Destroy => {
                destroy(&qemu, &lab_name, &lab_id)?;
            }
            Commands::Inspect => {
                inspect(&qemu, &lab_name, &lab_id, &manifest.devices)?;
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
                clean(&qemu, *all, *disks, *networks, &lab_id)?;
            }
            Commands::Console { name } => {
                console(name, &manifest)?;
            }
            Commands::Ssh { name } => {
                ssh(&qemu, name, &lab_name, &lab_id)?;
            }
        }
        Ok(())
    }
}
