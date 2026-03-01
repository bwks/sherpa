use anyhow::{Context, Result, bail};
use clap::Subcommand;

use crate::cmd::cli::OutputFormat;
use crate::common::rpc::RpcClient;
use crate::token;
use shared::data::{self, NodeKind, NodeModel, ServerConnection};
use shared::util::{emoji_success, render_images_table, render_scanned_images_table};

#[derive(Debug, Subcommand)]
pub enum ImageCommands {
    /// List all images
    List {
        /// Optional: List images for a specific model
        #[arg(value_enum)]
        model: Option<NodeModel>,
        /// List container images
        #[arg(long, action = clap::ArgAction::SetTrue)]
        container: bool,
        /// List unikernel images
        #[arg(long, action = clap::ArgAction::SetTrue)]
        unikernel: bool,
        /// List virtual machine images
        #[arg(long, action = clap::ArgAction::SetTrue)]
        virtual_machine: bool,
    },

    /// Scan the images directory for on-disk images and import to database
    Scan {
        /// Scan container images
        #[arg(long, action = clap::ArgAction::SetTrue)]
        container: bool,
        /// Scan unikernel images
        #[arg(long, action = clap::ArgAction::SetTrue)]
        unikernel: bool,
        /// Scan virtual machine images
        #[arg(long, action = clap::ArgAction::SetTrue)]
        virtual_machine: bool,
        /// Show what would be imported without making changes
        #[arg(long, action = clap::ArgAction::SetTrue)]
        dry_run: bool,
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
        model: NodeModel,
    },

    /// Pull a container image from an OCI registry
    Pull {
        /// Container image reference (e.g., ghcr.io/nokia/srlinux:1.2.3)
        image: String,
    },
}

pub async fn image_commands(
    command: &ImageCommands,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    match command {
        ImageCommands::List {
            model,
            container,
            unikernel,
            virtual_machine,
        } => {
            let kind = if *container {
                Some(NodeKind::Container)
            } else if *unikernel {
                Some(NodeKind::Unikernel)
            } else if *virtual_machine {
                Some(NodeKind::VirtualMachine)
            } else {
                None
            };
            list_images(*model, kind, server_url, server_connection, output_format).await
        }
        ImageCommands::Scan {
            container,
            unikernel,
            virtual_machine,
            dry_run,
        } => {
            let kind = if *container {
                Some(NodeKind::Container)
            } else if *unikernel {
                Some(NodeKind::Unikernel)
            } else if *virtual_machine {
                Some(NodeKind::VirtualMachine)
            } else {
                None
            };
            scan_images(kind, *dry_run, server_url, server_connection, output_format).await
        }
        ImageCommands::Import {
            src,
            version,
            model,
        } => {
            import_image(
                src,
                version,
                model,
                server_url,
                server_connection,
                output_format,
            )
            .await
        }
        ImageCommands::Pull { image } => {
            pull_image(image, server_url, server_connection, output_format).await
        }
    }
}

async fn list_images(
    model: Option<NodeModel>,
    kind: Option<NodeKind>,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    let token = token::load_token().context("Not authenticated. Please login first.")?;

    let request = data::ListImagesRequest { model, kind };

    let rpc_client = RpcClient::new(server_url.to_string(), server_connection.clone());
    let response: data::ListImagesResponse = rpc_client
        .call("image.list", request, Some(token))
        .await
        .context("Failed to list images")?;

    match output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        OutputFormat::Text => {
            if response.images.is_empty() {
                println!("No images found");
            } else {
                println!("\n{}", render_images_table(&response.images));
            }
        }
    }

    Ok(())
}

async fn scan_images(
    kind: Option<NodeKind>,
    dry_run: bool,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    let token = token::load_token().context("Not authenticated. Please login first.")?;

    let request = data::ScanImagesRequest { kind, dry_run };

    let rpc_client = RpcClient::new(server_url.to_string(), server_connection.clone());
    let response: data::ScanImagesResponse = rpc_client
        .call("image.scan", request, Some(token))
        .await
        .context("Failed to scan images")?;

    match output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        OutputFormat::Text => {
            if response.scanned.is_empty() {
                println!("No images found on disk");
            } else {
                println!("\n{}", render_scanned_images_table(&response.scanned));
                if dry_run {
                    println!(
                        "\nDry run: {} images would be imported (no changes made)",
                        response.total_imported
                    );
                } else {
                    println!(
                        "\nScan complete: {} images found, {} imported",
                        response.scanned.len(),
                        response.total_imported
                    );
                }
            }
        }
    }

    Ok(())
}

async fn import_image(
    src: &str,
    version: &str,
    model: &NodeModel,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    let token = token::load_token().context("Not authenticated. Please login first.")?;

    let request = data::ImportRequest {
        model: *model,
        version: version.to_string(),
        src: src.to_string(),
    };

    let rpc_client = RpcClient::new(server_url.to_string(), server_connection.clone());
    let response: data::ImportResponse = rpc_client
        .call("image.import", request, Some(token))
        .await
        .context("Failed to import image")?;

    match output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        OutputFormat::Text => {
            if response.success {
                println!("{}", emoji_success("Image imported successfully"));
                println!("   Model:    {}", response.model);
                println!("   Kind:     {}", response.kind);
                println!("   Version:  {}", response.version);
                println!("   Path:     {}", response.image_path);
                println!(
                    "   DB Track: {}",
                    if response.db_tracked { "yes" } else { "no" }
                );
            } else {
                eprintln!("Image import failed");
            }
        }
    }

    Ok(())
}

async fn pull_image(
    image: &str,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    let token = token::load_token().context("Not authenticated. Please login first.")?;

    // Parse image reference into repo:tag
    let (repo, tag) = if let Some((r, t)) = image.rsplit_once(':') {
        (r.to_string(), t.to_string())
    } else {
        bail!("Invalid image format. Expected format: repo:tag (e.g., ghcr.io/nokia/srlinux:1.2.3)")
    };

    let request = data::ContainerPullRequest {
        repo: repo.clone(),
        tag: tag.clone(),
    };

    let rpc_client = RpcClient::new(server_url.to_string(), server_connection.clone());
    let response: data::ContainerPullResponse = rpc_client
        .call("image.pull", request, Some(token))
        .await
        .context("Failed to pull container image")?;

    match output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        OutputFormat::Text => {
            if response.success {
                println!(
                    "{}",
                    emoji_success(&format!(
                        "Container image {}:{} pulled successfully",
                        response.repo, response.tag
                    ))
                );
            } else {
                eprintln!("Container image pull failed: {}", response.message);
            }
        }
    }

    Ok(())
}
