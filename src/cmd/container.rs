use anyhow::{bail, Result};
use clap::Subcommand;

use crate::core::konst::{
    CONTAINER_DHCP4_REPO, CONTAINER_DISK_NAME, CONTAINER_DNS_REPO, CONTAINER_IMAGE_NAME,
    CONTAINER_TFTPD_REPO, CONTAINER_WEBDIR_REPO, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_EXT4_1G,
    SHERPA_BLANK_DISK_EXT4_2G, SHERPA_BLANK_DISK_EXT4_3G, SHERPA_BLANK_DISK_EXT4_4G,
    SHERPA_BLANK_DISK_EXT4_5G, TEMP_DIR,
};
use crate::core::{Config, Sherpa};
use crate::data::{ContainerImage, ContainerModel, DeviceModels};
use crate::util::{
    check_file_size, copy_file, copy_to_ext4_image, create_dir, create_symlink, delete_dirs,
    dir_exists, file_exists, fix_permissions_recursive, pull_container_image, save_container_image,
    term_msg_surround,
};

#[derive(Debug, Subcommand)]
#[group(
    id = "image_selector",
    args = ["name", "model"],
    required = true
)]
pub enum ContainerCommands {
    /// Pull a container image from an image hosting service.
    Pull {
        /// Image name - srlinux
        #[arg(short, long, requires_all = ["repo", "version"])]
        name: Option<String>,
        /// Image Repository - ghcr.io/nokia/srlinux
        #[arg(short, long)]
        repo: Option<String>,
        /// Image version - 1.2.3
        #[arg(short, long)]
        version: Option<String>,
        #[arg(short, long)]
        /// Container Model
        model: Option<ContainerModel>,
    },
    /// Import a local container image as a Sherpa box.
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

pub async fn parse_container_commands(commands: &ContainerCommands, sherpa: &Sherpa) -> Result<()> {
    let config = Config::load(&sherpa.config_path)?;
    match commands {
        ContainerCommands::Pull {
            name,
            repo,
            version,
            model,
        } => {
            let container_image = match model {
                Some(model) => match model {
                    ContainerModel::Tftpd => ContainerImage::tftpd(),
                    ContainerModel::Webdir => ContainerImage::webdir(),
                    ContainerModel::Dns => ContainerImage::dns(),
                    ContainerModel::Dhcp4 => ContainerImage::dhcp4(),
                    ContainerModel::Srlinux => ContainerImage::srlinux(),
                },
                None => ContainerImage {
                    // These values should always be set if
                    // model was not provided as an argument.
                    name: name.clone().unwrap().to_owned(),
                    repo: repo.clone().unwrap().to_owned(),
                    version: version.clone().unwrap().to_owned(),
                },
            };

            pull_container_image(&config, &container_image).await?;
        }
        ContainerCommands::Import {
            image,
            version,
            model,
            latest,
        } => {
            term_msg_surround("Importing container image");

            if !dir_exists(TEMP_DIR) {
                create_dir(TEMP_DIR)?;
            }

            save_container_image(image, version)?;

            let container_path = format!("{TEMP_DIR}/{CONTAINER_IMAGE_NAME}");

            if !file_exists(&container_path) {
                anyhow::bail!("File does not exist: {}", container_path);
            }

            let data_disk_base = match check_file_size(&container_path)? {
                1 => SHERPA_BLANK_DISK_EXT4_1G,
                2 => SHERPA_BLANK_DISK_EXT4_2G,
                3 => SHERPA_BLANK_DISK_EXT4_3G,
                4 => SHERPA_BLANK_DISK_EXT4_4G,
                5 => SHERPA_BLANK_DISK_EXT4_5G,
                _ => bail!("Container image is larger than 5GB and not supported."),
            };

            // Copy a blank disk to to .tmp directory
            let src_data_disk = format!(
                "{}/{}/{}",
                &sherpa.boxes_dir, SHERPA_BLANK_DISK_DIR, data_disk_base
            );
            let dst_data_disk = format!("{TEMP_DIR}/{CONTAINER_DISK_NAME}");

            copy_file(&src_data_disk, &dst_data_disk)?;

            // Copy to container image into the container disk
            copy_to_ext4_image(vec![&container_path], &dst_data_disk, "/")?;

            let dst_path = format!("{}/{}", &sherpa.boxes_dir, model);
            let dst_version_dir = format!("{dst_path}/{version}");
            let dst_latest_dir = format!("{dst_path}/latest");

            create_dir(&dst_version_dir)?;
            create_dir(&dst_latest_dir)?;

            let dst_version_disk = format!("{dst_version_dir}/{CONTAINER_DISK_NAME}");

            if !file_exists(&dst_version_disk) {
                println!(
                    "Copying file from: {} to: {}",
                    &dst_data_disk, dst_version_disk
                );
                copy_file(&dst_data_disk, &dst_version_disk)?;
                println!(
                    "Copied file from: {} to: {}",
                    &dst_data_disk, dst_version_disk
                );
            } else {
                println!("File already exists: {}", dst_version_disk);
            }

            if *latest {
                let dst_latest_disk = format!("{dst_latest_dir}/{CONTAINER_DISK_NAME}");
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

            // Delete the local .tmp directory
            delete_dirs(TEMP_DIR)?;
        }
    }
    Ok(())
}
