use anyhow::{Result, bail};
use clap::Subcommand;

use container::{docker_connection, list_images, pull_image};
use data::Sherpa;
use util::{load_config, term_msg_surround};

#[derive(Debug, Subcommand)]
#[group(
    id = "image_selector",
    args = ["name", "model"],
    required = true
)]
pub enum ContainerCommands {
    /// Manage container images
    Image {
        #[command(subcommand)]
        command: ImageCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum ImageCommands {
    /// List container images
    List,
    /// Pull a container image from an image hosting service.
    Pull {
        /// Container image reference (e.g., nginx:1.29.4-perl, ghcr.io/nokia/srlinux:1.2.3)
        image: String,
    },
    // /// Import a local container image as a Sherpa box.
    // Import {
    //     /// Source container image
    //     #[arg(short, long)]
    //     image: String,
    //     /// Version of the device model
    //     #[arg(short, long)]
    //     version: String,
    //     /// Model of Device
    //     #[arg(short, long, value_enum)]
    //     model: NodeModel,
    //     /// Import the container image as the latest version
    //     #[arg(long, action = clap::ArgAction::SetTrue)]
    //     latest: bool,
    // },
}

pub async fn parse_container_commands(commands: &ContainerCommands, sherpa: &Sherpa) -> Result<()> {
    let _config = load_config(&sherpa.config_file_path)?;
    match commands {
        ContainerCommands::Image { command } => match command {
            ImageCommands::List => {
                term_msg_surround("Container images");
                let docker_conn = docker_connection()?;
                list_images(&docker_conn).await?;
            }
            ImageCommands::Pull { image } => {
                // Parse the image reference to extract repo and version
                let (repo, version) = if let Some((r, v)) = image.rsplit_once(':') {
                    (r.to_string(), v.to_string())
                } else {
                    bail!(
                        "Invalid image format. Expected format: repo:version (e.g., nginx:1.29.4-perl)"
                    )
                };
                pull_image(&repo, &version).await?;
            } //
        },
    }
    Ok(())
}
