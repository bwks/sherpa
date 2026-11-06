use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Subcommand};

use shared::data::{self, NodeConfig, NodeKind, NodeModel, ServerConnection, StatusMessage};
use shared::konst::{
    CONTAINER_FRR_REPO, CONTAINER_GITLAB_CE_REPO, CONTAINER_HASHICORP_VAULT_REPO,
    CONTAINER_KEYCLOAK_REPO, CONTAINER_MONGO_DB_REPO, CONTAINER_NOKIA_SRLINUX_REPO,
};
use shared::util::{
    emoji_success, render_image_detail_table, render_images_table, render_scanned_images_table,
};

use super::OutputFormat;
use super::{rpc_call, rpc_call_streaming};
use crate::cmd::progress::status_emoji;

/// Which kind of image an image command works on: at most one may be given, and none means
/// every kind.
#[derive(Debug, Args)]
#[group(multiple = false)]
pub struct ImageKindFlags {
    /// Only container images
    #[arg(long)]
    container: bool,
    /// Only unikernel images
    #[arg(long)]
    unikernel: bool,
    /// Only virtual machine images
    #[arg(long)]
    virtual_machine: bool,
}

impl ImageKindFlags {
    /// The kind of image given, if any.
    pub fn kind(&self) -> Option<NodeKind> {
        if self.container {
            Some(NodeKind::Container)
        } else if self.unikernel {
            Some(NodeKind::Unikernel)
        } else if self.virtual_machine {
            Some(NodeKind::VirtualMachine)
        } else {
            None
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum ServerImageCommands {
    /// Show detailed information about an image
    Show {
        /// Model of the device image
        #[arg(value_enum)]
        model: NodeModel,
        /// Optional: show a specific version (defaults to the default version)
        #[arg(long)]
        version: Option<String>,
    },

    /// List all images
    List {
        /// Optional: List images for a specific model
        #[arg(value_enum)]
        model: Option<NodeModel>,
        #[command(flatten)]
        kind: ImageKindFlags,
    },

    /// Scan the images directory for on-disk images and import to database
    Scan {
        #[command(flatten)]
        kind: ImageKindFlags,
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
        /// Set this image as the default version
        #[arg(long, action = clap::ArgAction::SetTrue)]
        default: bool,
    },

    /// Pull a container or VM image
    Pull {
        /// Model of the device image to pull
        #[arg(short, long, value_enum)]
        model: NodeModel,
        /// Version of the image (required for known-repo containers and VMs)
        #[arg(short, long)]
        version: Option<String>,
        /// Full container image reference (e.g., registry.io/account/image:version)
        #[arg(short, long)]
        repo: Option<String>,
        /// VM image download URL
        #[arg(short, long)]
        url: Option<String>,
        /// Set this image as the default version
        #[arg(long, action = clap::ArgAction::SetTrue)]
        default: bool,
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
        ServerImageCommands::Show { model, version } => {
            show_image(
                *model,
                version.clone(),
                server_url,
                server_connection,
                output_format,
            )
            .await
        }
        ServerImageCommands::List { model, kind } => {
            list_images(
                *model,
                kind.kind(),
                server_url,
                server_connection,
                output_format,
            )
            .await
        }
        ServerImageCommands::Scan { kind, dry_run } => {
            let kind = kind.kind();
            scan_images(kind, *dry_run, server_url, server_connection, output_format).await
        }
        ServerImageCommands::Import {
            src,
            version,
            model,
            default,
        } => {
            import_image(
                src,
                version,
                model,
                *default,
                server_url,
                server_connection,
                output_format,
            )
            .await
        }
        ServerImageCommands::Pull {
            model,
            version,
            repo,
            url,
            default,
        } => {
            let config = NodeConfig::get_model(*model);
            match config.kind {
                NodeKind::Container => {
                    pull_container_image(
                        model,
                        version.as_deref(),
                        repo.as_deref(),
                        *default,
                        server_url,
                        server_connection,
                        output_format,
                    )
                    .await
                }
                NodeKind::VirtualMachine => {
                    let version = version.as_deref().ok_or_else(|| {
                        anyhow::anyhow!("--version is required for VM image downloads")
                    })?;
                    download_vm_image(
                        model,
                        version,
                        url.as_deref(),
                        *default,
                        server_url,
                        server_connection,
                        output_format,
                    )
                    .await
                }
                NodeKind::Unikernel => {
                    bail!("Image pull is not supported for unikernel models")
                }
            }
        }
        ServerImageCommands::Delete { model, version } => {
            delete_image(model, version, server_url, server_connection, output_format).await
        }
        ServerImageCommands::SetDefault { model, version } => {
            set_default_image(model, version, server_url, server_connection, output_format).await
        }
    }
}

pub async fn list_images(
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

pub async fn show_image(
    model: NodeModel,
    version: Option<String>,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    let request = data::ShowImageRequest { model, version };

    let response: data::ShowImageResponse =
        rpc_call("image.show", request, server_url, server_connection)
            .await
            .context("Failed to show image")?;

    match output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        OutputFormat::Text => {
            println!("\n{}", render_image_detail_table(&response.image));
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
    default: bool,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    let request = data::ImportRequest {
        model: *model,
        version: version.to_string(),
        src: src.to_string(),
        default,
    };

    let response: data::ImportResponse = rpc_call_streaming(
        "image.import",
        request,
        server_url,
        server_connection,
        print_status,
    )
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
                println!("   Default:  {}", if default { "yes" } else { "no" });
            }
        }
    }

    if !response.success {
        bail!("Image import failed");
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
            }
        }
    }

    if !response.success {
        bail!("Image delete failed");
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
            }
        }
    }

    if !response.success {
        bail!("Failed to set default image");
    }

    Ok(())
}

async fn pull_container_image(
    model: &NodeModel,
    version: Option<&str>,
    repo: Option<&str>,
    default: bool,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    let (pull_repo, pull_tag) = if let Some(reference) = repo {
        let (repo, tag) = split_image_reference(reference)?;
        (repo.to_string(), tag.to_string())
    } else {
        // Known public repos — resolve from constants, version is the tag
        let known_repo = match model {
            NodeModel::NokiaSrlinux => Some(CONTAINER_NOKIA_SRLINUX_REPO),
            NodeModel::FrrLinux => Some(CONTAINER_FRR_REPO),
            NodeModel::GitlabCe => Some(CONTAINER_GITLAB_CE_REPO),
            NodeModel::HashicorpVault => Some(CONTAINER_HASHICORP_VAULT_REPO),
            NodeModel::KeycloakServer => Some(CONTAINER_KEYCLOAK_REPO),
            NodeModel::MongoDb => Some(CONTAINER_MONGO_DB_REPO),
            _ => None,
        };

        if let Some(repo) = known_repo {
            let tag = version
                .ok_or_else(|| anyhow::anyhow!("--version is required for model '{model}'"))?;
            (repo.to_string(), tag.to_string())
        } else {
            bail!(
                "--repo is required for model '{model}'. Provide the full image reference (e.g., registry.io/account/image:version)"
            )
        }
    };

    let request = data::ContainerPullRequest {
        model: *model,
        repo: pull_repo.clone(),
        tag: pull_tag.clone(),
        default,
    };

    let response: data::ContainerPullResponse = rpc_call_streaming(
        "image.pull",
        request,
        server_url,
        server_connection,
        print_status,
    )
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
                println!(
                    "   DB Track: {}",
                    if response.db_tracked { "yes" } else { "no" }
                );
            }
        }
    }

    if !response.success {
        bail!("Container image pull failed: {}", response.message);
    }

    Ok(())
}

async fn download_vm_image(
    model: &NodeModel,
    version: &str,
    url: Option<&str>,
    default: bool,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    let request = data::DownloadImageRequest {
        model: *model,
        version: version.to_string(),
        url: url.map(std::string::ToString::to_string),
        default,
    };

    let response: data::ImportResponse = rpc_call_streaming(
        "image.download",
        request,
        server_url,
        server_connection,
        print_status,
    )
    .await
    .context("Failed to download VM image")?;

    match output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        OutputFormat::Text => {
            if response.success {
                println!("{}", emoji_success("VM image downloaded successfully"));
                println!("   Model:    {}", response.model);
                println!("   Kind:     {}", response.kind);
                println!("   Version:  {}", response.version);
                println!("   Path:     {}", response.image_path);
                println!(
                    "   DB Track: {}",
                    if response.db_tracked { "yes" } else { "no" }
                );
                println!("   Default:  {}", if default { "yes" } else { "no" });
            }
        }
    }

    if !response.success {
        bail!("VM image download failed");
    }

    Ok(())
}

/// The repository and tag of an image `reference` written as `registry/image:tag`. The tag
/// follows the last `:` and cannot contain a `/`, so a registry port such as
/// `localhost:5000/img` is not taken for a tag.
fn split_image_reference(reference: &str) -> Result<(&str, &str)> {
    let invalid = || {
        format!(
            "Image reference '{reference}' must be registry/image:tag, such as \
             ghcr.io/nokia/srlinux:1.2.3; digests (@sha256:...) are not supported"
        )
    };
    let (repo, tag) = reference.rsplit_once(':').with_context(invalid)?;
    ensure!(
        !repo.is_empty() && !tag.is_empty() && !tag.contains('/') && !reference.contains('@'),
        invalid()
    );
    Ok((repo, tag))
}

/// Show a status message the server streams during a long image operation. It goes to stderr,
/// so with `--output json` stdout holds only the JSON result.
fn print_status(msg_text: &str) {
    if let Ok(status_msg) = serde_json::from_str::<StatusMessage>(msg_text)
        && status_msg.r#type == "status"
    {
        eprintln!("{} {}", status_emoji(status_msg.kind), status_msg.message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_reference_splits_at_its_tag() {
        assert_eq!(
            split_image_reference("ghcr.io/nokia/srlinux:1.2.3").unwrap(),
            ("ghcr.io/nokia/srlinux", "1.2.3")
        );
        assert_eq!(
            split_image_reference("localhost:5000/img:1").unwrap(),
            ("localhost:5000/img", "1")
        );
    }

    #[test]
    fn an_image_reference_without_a_tag_is_refused() {
        for reference in ["localhost:5000/img", "img", "img:", ":1", "img@sha256:abc"] {
            assert!(split_image_reference(reference).is_err(), "{reference}");
        }
    }
}
