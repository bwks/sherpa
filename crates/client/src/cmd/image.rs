use anyhow::Result;
use clap::Subcommand;

use super::server::{OutputFormat, list_images, show_image};

use shared::data::{ClientConfig, NodeKind, NodeModel, ShowImageRequest};

#[derive(Debug, Subcommand)]
pub enum ImageCommands {
    /// Show detailed information about an image
    Show {
        /// Model of the device image
        #[arg(value_enum)]
        model: NodeModel,
        /// Kind of the image. Without it, the model's default kind
        #[arg(long, value_enum)]
        kind: Option<NodeKind>,
        /// Optional: show a specific version (defaults to the default version)
        #[arg(long)]
        version: Option<String>,
    },

    /// List all images
    List {
        /// Optional: List all images for a model
        #[arg(value_enum)]
        model: Option<NodeModel>,
        /// Only images of this kind; without it, every kind
        #[arg(long, value_enum)]
        kind: Option<NodeKind>,
    },
}

/// Run an image command. These are `sherpa server image show` and `list` with text output.
pub async fn parse_image_commands(
    commands: &ImageCommands,
    config: &ClientConfig,
    server_url: &str,
) -> Result<()> {
    let connection = &config.server_connection;
    match commands {
        ImageCommands::Show {
            model,
            kind,
            version,
        } => {
            let request = ShowImageRequest {
                model: *model,
                kind: *kind,
                version: version.clone(),
            };
            show_image(request, server_url, connection, &OutputFormat::Text).await
        }
        ImageCommands::List { model, kind } => {
            list_images(*model, *kind, server_url, connection, &OutputFormat::Text).await
        }
    }
}
