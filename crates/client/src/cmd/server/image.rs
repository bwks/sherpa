use anyhow::{Context, Result, bail};
use clap::Subcommand;

use shared::data::{self, NodeKind, NodeModel, ServerConnection};
use shared::util::{emoji_success, render_images_table, render_scanned_images_table};

use super::OutputFormat;
use super::rpc_call;

#[derive(Debug, Subcommand)]
pub enum ServerImageCommands {
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

    /// Delete an imported image from disk and database
    Delete {
        /// Model of the device image to delete
        #[arg(short, long, value_enum)]
        model: NodeModel,
        /// Version of the device image to delete
        #[arg(short, long)]
        version: String,
    },

    /// Set the default version for an image
    SetDefault {
        /// Model of the device image
        #[arg(short, long, value_enum)]
        model: NodeModel,
        /// Version to set as default
        #[arg(short, long)]
        version: String,
    },
}

pub async fn image_commands(
    command: &ServerImageCommands,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    match command {
        ServerImageCommands::List {
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
        ServerImageCommands::Scan {
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
        ServerImageCommands::Import {
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
        ServerImageCommands::Pull { image } => {
            pull_image(image, server_url, server_connection, output_format).await
        }
        ServerImageCommands::Delete { model, version } => {
            delete_image(model, version, server_url, server_connection, output_format).await
        }
        ServerImageCommands::SetDefault { model, version } => {
            set_default_image(model, version, server_url, server_connection, output_format).await
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
    let request = data::ListImagesRequest { model, kind };

    let response: data::ListImagesResponse =
        rpc_call("image.list", request, server_url, server_connection)
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
    let request = data::ScanImagesRequest { kind, dry_run };

    let response: data::ScanImagesResponse =
        rpc_call("image.scan", request, server_url, server_connection)
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
    let request = data::ImportRequest {
        model: *model,
        version: version.to_string(),
        src: src.to_string(),
    };

    let response: data::ImportResponse =
        rpc_call("image.import", request, server_url, server_connection)
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

async fn delete_image(
    model: &NodeModel,
    version: &str,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    let request = data::DeleteImageRequest {
        model: *model,
        version: version.to_string(),
    };

    let response: data::DeleteImageResponse =
        rpc_call("image.delete", request, server_url, server_connection)
            .await
            .context("Failed to delete image")?;

    match output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        OutputFormat::Text => {
            if response.success {
                println!("{}", emoji_success("Image deleted successfully"));
                println!("   Model:       {}", response.model);
                println!("   Kind:        {}", response.kind);
                println!("   Version:     {}", response.version);
                println!(
                    "   DB Deleted:  {}",
                    if response.db_deleted { "yes" } else { "no" }
                );
                println!(
                    "   Disk Deleted: {}",
                    if response.disk_deleted { "yes" } else { "no" }
                );
            } else {
                eprintln!("Image delete failed");
            }
        }
    }

    Ok(())
}

async fn set_default_image(
    model: &NodeModel,
    version: &str,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    let request = data::SetDefaultImageRequest {
        model: *model,
        version: version.to_string(),
    };

    let response: data::SetDefaultImageResponse =
        rpc_call("image.set_default", request, server_url, server_connection)
            .await
            .context("Failed to set default image")?;

    match output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        OutputFormat::Text => {
            if response.success {
                println!("{}", emoji_success("Default image set successfully"));
                println!("   Model:   {}", response.model);
                println!("   Kind:    {}", response.kind);
                println!("   Version: {}", response.version);
            } else {
                eprintln!("Failed to set default image");
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

    let response: data::ContainerPullResponse =
        rpc_call("image.pull", request, server_url, server_connection)
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
