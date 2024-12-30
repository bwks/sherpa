use anyhow::Result;
use clap::Subcommand;

use crate::core::konst::CONTAINER_IMAGE_NAME;
use crate::core::Sherpa;
use crate::data::DeviceModels;
use crate::util::{
    copy_file, create_dir, create_symlink, delete_file, file_exists, fix_permissions_recursive,
    pull_container_image, save_container_image, term_msg_surround,
};

#[derive(Debug, Subcommand)]
pub enum ContainerCommands {
    /// Pull a container image
    Pull {
        /// Container image reference (e.g., alpine:latest)
        #[arg(short, long)]
        image: String,
    },
    /// Import a container image
    Import {
        /// Source container image
        #[arg(short, long)]
        image: String,
        /// Version of the device model
        #[arg(short, long)]
        version: String,
        /// Model of Device
        #[arg(short, long, value_enum)]
        model: DeviceModels,
        /// Import the container image as the latest version
        #[arg(long, action = clap::ArgAction::SetTrue)]
        latest: bool,
    },
}

pub fn parse_container_commands(commands: &ContainerCommands, sherpa: &Sherpa) -> Result<()> {
    match commands {
        ContainerCommands::Pull { image } => {
            pull_container_image(image)?;
        }
        ContainerCommands::Import {
            image,
            version,
            model,
            latest,
        } => {
            term_msg_surround("Importing container image");

            save_container_image(image, version)?;

            if !file_exists(CONTAINER_IMAGE_NAME) {
                anyhow::bail!("File does not exist: {}", CONTAINER_IMAGE_NAME);
            }

            let dst_path = format!("{}/{}", &sherpa.boxes_dir, model);
            let dst_version_dir = format!("{dst_path}/{version}");
            let dst_latest_dir = format!("{dst_path}/latest");

            create_dir(&dst_version_dir)?;
            create_dir(&dst_latest_dir)?;

            let dst_version_disk = format!("{dst_version_dir}/{CONTAINER_IMAGE_NAME}");

            if !file_exists(&dst_version_disk) {
                println!(
                    "Copying file from: {} to: {}",
                    CONTAINER_IMAGE_NAME, dst_version_disk
                );
                copy_file(CONTAINER_IMAGE_NAME, &dst_version_disk)?;
                println!(
                    "Copied file from: {} to: {}",
                    CONTAINER_IMAGE_NAME, dst_version_disk
                );
            } else {
                println!("File already exists: {}", dst_version_disk);
            }

            if *latest {
                let dst_latest_disk = format!("{dst_latest_dir}/{CONTAINER_IMAGE_NAME}");
                println!(
                    "Symlinking file from: {} to: {}",
                    dst_version_disk, dst_latest_disk
                );
                create_symlink(&dst_version_disk, &dst_latest_disk)?;
                println!(
                    "Symlinked file from: {} to: {}",
                    dst_version_disk, dst_latest_disk
                );
            }

            println!("Setting base box files to read-only");

            // Update box permissions
            fix_permissions_recursive(&sherpa.boxes_dir)?;

            // Delete the local image.tar.gz
            delete_file(CONTAINER_IMAGE_NAME)?;
        }
    }
    Ok(())
}
