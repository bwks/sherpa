use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Subcommand};

use shared::data::{self, ModelKind, NodeKind, NodeModel, ServerConnection, StatusMessage};
use shared::util::emoji_success;

use super::OutputFormat;
use super::{rpc_call, rpc_call_streaming};
use crate::cmd::outcome::ensure_images_recorded;
use crate::cmd::progress::status_emoji;
use crate::error::ClientError;
use crate::table::{
    render_image_detail_table, render_images_table, render_scanned_images_table,
    render_skipped_models_table,
};

/// What `image scan` scans for.
#[derive(Debug, Args)]
pub struct ScanArgs {
    /// Only images of this kind; without it, every kind
    #[arg(long, value_enum)]
    kind: Option<NodeKind>,
    /// Show what would be imported without making changes
    #[arg(long, action = clap::ArgAction::SetTrue)]
    dry_run: bool,
    /// Container model to record the local tags of --repo for (with --kind container)
    #[arg(short, long, requires_all = ["repo", "kind"])]
    model: Option<NodeModel>,
    /// Image repository without a tag (e.g., mysql, docker.io/library/mysql or
    /// ghcr.io/nokia/srlinux); each of its local tags is recorded as a version of --model
    #[arg(short, long, requires = "model")]
    repo: Option<String>,
}

impl ScanArgs {
    /// The scan request these arguments ask for.
    fn request(&self) -> Result<data::ScanImagesRequest> {
        ensure!(
            self.model.is_none() || self.kind == Some(NodeKind::Container),
            ClientError::ScanModelWithoutContainerKind
        );
        Ok(data::ScanImagesRequest {
            kind: self.kind,
            dry_run: self.dry_run,
            model: self.model,
            repo: self.repo.clone(),
        })
    }
}

/// What `image pull` pulls.
#[derive(Debug, Args)]
pub struct PullArgs {
    /// Model of the device image to pull
    #[arg(short, long)]
    model: NodeModel,
    /// Kind of the image, which decides between a registry pull and a download. Without it,
    /// the model's default kind
    #[arg(long, value_enum)]
    kind: Option<NodeKind>,
    /// Version of the image: the tag pulled from the model's built-in repo when --repo is left
    /// out, and required for VMs
    #[arg(short, long)]
    version: Option<String>,
    /// Container image reference with its tag (e.g., nats:2.11 or
    /// registry.io/account/image:version). Without it, the server's built-in repo for the model
    #[arg(short, long)]
    repo: Option<String>,
    /// VM image download URL
    #[arg(short, long)]
    url: Option<String>,
    /// Set this image as the default version
    #[arg(long, action = clap::ArgAction::SetTrue)]
    default: bool,
}

#[derive(Debug, Subcommand)]
pub enum ServerImageCommands {
    /// Show detailed information about an image
    Show {
        /// Model of the device image
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
        /// Optional: List images for a specific model
        model: Option<NodeModel>,
        /// Only images of this kind; without it, every kind
        #[arg(long, value_enum)]
        kind: Option<NodeKind>,
    },

    /// Record images that are present but not recorded: disk images in the images directory
    /// and container images in local Docker
    Scan(ScanArgs),

    /// Import a disk image
    Import {
        /// Source path of the disk image
        #[arg(short, long)]
        src: String,
        /// Version of the device model
        #[arg(short, long)]
        version: String,
        /// Model of Device
        #[arg(short, long)]
        model: NodeModel,
        /// Kind of the image. Without it, the model's default kind
        #[arg(long, value_enum)]
        kind: Option<NodeKind>,
        /// Set this image as the default version
        #[arg(long, action = clap::ArgAction::SetTrue)]
        default: bool,
    },

    /// Pull a container or VM image
    Pull(PullArgs),

    /// Delete an imported image from disk and database
    Delete {
        /// Model of the device image to delete
        #[arg(short, long)]
        model: NodeModel,
        /// Kind of the image. Without it, the model's default kind
        #[arg(long, value_enum)]
        kind: Option<NodeKind>,
        /// Version of the device image to delete
        #[arg(short, long)]
        version: String,
    },

    /// Set the default version for an image
    SetDefault {
        /// Model of the device image
        #[arg(short, long)]
        model: NodeModel,
        /// Kind of the image. Without it, the model's default kind
        #[arg(long, value_enum)]
        kind: Option<NodeKind>,
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
        ServerImageCommands::Show {
            model,
            kind,
            version,
        } => {
            let request = data::ShowImageRequest {
                model: *model,
                kind: *kind,
                version: version.clone(),
            };
            show_image(request, server_url, server_connection, output_format).await
        }
        ServerImageCommands::List { model, kind } => {
            list_images(*model, *kind, server_url, server_connection, output_format).await
        }
        ServerImageCommands::Scan(args) => {
            scan_images(
                args.request()?,
                server_url,
                server_connection,
                output_format,
            )
            .await
        }
        ServerImageCommands::Import {
            src,
            version,
            model,
            kind,
            default,
        } => {
            let request = data::ImportRequest {
                model: *model,
                kind: *kind,
                version: version.clone(),
                src: src.clone(),
                default: *default,
            };
            import_image(request, server_url, server_connection, output_format).await
        }
        ServerImageCommands::Pull(args) => {
            pull_image(args, server_url, server_connection, output_format).await
        }
        ServerImageCommands::Delete {
            model,
            kind,
            version,
        } => {
            let request = data::DeleteImageRequest {
                model: *model,
                kind: *kind,
                version: version.clone(),
            };
            delete_image(request, server_url, server_connection, output_format).await
        }
        ServerImageCommands::SetDefault {
            model,
            kind,
            version,
        } => {
            let request = data::SetDefaultImageRequest {
                model: *model,
                kind: *kind,
                version: version.clone(),
            };
            set_default_image(request, server_url, server_connection, output_format).await
        }
    }
}

/// Pull a container image from a registry, or download a VM image, as the image's kind says.
async fn pull_image(
    args: &PullArgs,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    let model_kind = ModelKind::new(args.model, args.kind)?;
    match model_kind.kind() {
        NodeKind::Container => {
            pull_container_image(
                model_kind,
                args.version.as_deref(),
                args.repo.as_deref(),
                args.default,
                server_url,
                server_connection,
                output_format,
            )
            .await
        }
        NodeKind::VirtualMachine => {
            let version = args
                .version
                .as_deref()
                .context("--version is required for VM image downloads")?;
            download_vm_image(
                model_kind,
                version,
                args.url.as_deref(),
                args.default,
                server_url,
                server_connection,
                output_format,
            )
            .await
        }
        NodeKind::Unikernel => bail!("Image pull is not supported for unikernel models"),
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
    request: data::ShowImageRequest,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
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
    request: data::ScanImagesRequest,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    let dry_run = request.dry_run;
    let repo = request.repo.clone();
    let response: data::ScanImagesResponse =
        rpc_call("image.scan", request, server_url, server_connection)
            .await
            .context("Failed to scan images")?;

    match output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        OutputFormat::Text => {
            if !response.skipped.is_empty() {
                println!("\n{}", render_skipped_models_table(&response.skipped));
                println!(
                    "Local Docker images are not matched to models with no built-in repo. \
                     Record a repo's images for one with:\n  sherpa server image scan \
                     --kind container --model <model> --repo <repo>"
                );
            }
            if response.scanned.is_empty() {
                match repo {
                    Some(repo) => println!(
                        "\nNo local Docker images found for repo {repo}. Docker matches the \
                         repo as `docker images` does: give a Docker Hub image by its short \
                         name (mysql) or as docker.io/library/<name>"
                    ),
                    None => println!("\nNo images found"),
                }
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

    ensure_images_recorded(&response.scanned)
}

async fn import_image(
    request: data::ImportRequest,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
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
                println!(
                    "   Default:  {}",
                    if response.default { "yes" } else { "no" }
                );
            }
        }
    }

    if !response.success {
        bail!("Image import failed");
    }

    Ok(())
}

async fn delete_image(
    request: data::DeleteImageRequest,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
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
                let image_label = match response.kind {
                    data::NodeKind::Container => "Docker Image Deleted",
                    data::NodeKind::VirtualMachine | data::NodeKind::Unikernel => "Disk Deleted",
                };
                println!(
                    "   {image_label}: {}",
                    if response.image_deleted { "yes" } else { "no" }
                );
                if let Some(new_default) = &response.new_default {
                    println!("   New Default: {new_default}");
                }
            }
        }
    }

    if !response.success {
        bail!("Image delete failed");
    }

    Ok(())
}

async fn set_default_image(
    request: data::SetDefaultImageRequest,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
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
    model_kind: ModelKind,
    version: Option<&str>,
    repo: Option<&str>,
    default: bool,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    let (repo, tag) = if let Some(reference) = repo {
        let (repo, tag) = split_image_reference(reference)?;
        (Some(repo.to_string()), tag.to_string())
    } else {
        let tag = version.context(
            "--version is required without --repo: it is the tag pulled from the model's \
             built-in repo",
        )?;
        (None, tag.to_string())
    };

    let request = data::ContainerPullRequest {
        model: model_kind.model(),
        kind: Some(model_kind.kind()),
        repo,
        tag,
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
    model_kind: ModelKind,
    version: &str,
    url: Option<&str>,
    default: bool,
    server_url: &str,
    server_connection: &ServerConnection,
    output_format: &OutputFormat,
) -> Result<()> {
    let request = data::DownloadImageRequest {
        model: model_kind.model(),
        kind: Some(model_kind.kind()),
        version: version.to_string(),
        url: url.map(ToString::to_string),
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
                println!(
                    "   Default:  {}",
                    if response.default { "yes" } else { "no" }
                );
            }
        }
    }

    if !response.success {
        bail!("VM image download failed");
    }

    Ok(())
}

/// The repository and tag of an image `reference` written as `image:tag` or
/// `registry/image:tag`. The tag follows the last `:` and cannot contain a `/`, so a registry
/// port such as `localhost:5000/img` is not taken for a tag. The repository is kept as written:
/// Docker resolves a short Docker Hub name such as `nats` itself.
fn split_image_reference(reference: &str) -> Result<(&str, &str)> {
    let invalid = || ClientError::InvalidImageReference {
        reference: reference.to_owned(),
    };
    let (repo, tag) = reference.rsplit_once(':').ok_or_else(invalid)?;
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
    use crate::cmd::Cli;
    use clap::Parser;

    #[test]
    fn image_pull_004_a_repo_is_split_at_its_tag() {
        assert_eq!(
            split_image_reference("nats:2.11").unwrap(),
            ("nats", "2.11")
        );
        assert_eq!(
            split_image_reference("docker.io/library/nats:2.11").unwrap(),
            ("docker.io/library/nats", "2.11")
        );
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
    fn image_pull_005_a_repo_without_a_tag_or_with_a_digest_is_refused_naming_the_forms_taken() {
        for reference in ["nats", "localhost:5000/img", "img:", ":1", "img@sha256:abc"] {
            let error = split_image_reference(reference).unwrap_err();
            assert!(
                matches!(
                    error.downcast_ref::<ClientError>(),
                    Some(ClientError::InvalidImageReference { reference: refused })
                        if refused == reference
                ),
                "{error:#}"
            );
        }
    }

    fn scan(kind: Option<NodeKind>, model: Option<NodeModel>) -> ScanArgs {
        ScanArgs {
            kind,
            dry_run: false,
            model,
            repo: model.map(|_| "mysql".to_string()),
        }
    }

    #[test]
    fn image_scan_012_the_cli_refuses_model_or_repo_alone_or_without_kind_container() {
        let request = scan(Some(NodeKind::Container), Some(NodeModel::MysqlDb))
            .request()
            .unwrap();
        assert_eq!(request.kind, Some(NodeKind::Container));
        assert_eq!(request.model, Some(NodeModel::MysqlDb));
        for kind in [NodeKind::VirtualMachine, NodeKind::Unikernel] {
            let error = scan(Some(kind), Some(NodeModel::MysqlDb))
                .request()
                .unwrap_err();
            assert!(
                matches!(
                    error.downcast_ref::<ClientError>(),
                    Some(ClientError::ScanModelWithoutContainerKind)
                ),
                "{error:#}"
            );
        }
        for kind in [
            None,
            Some(NodeKind::VirtualMachine),
            Some(NodeKind::Container),
            Some(NodeKind::Unikernel),
        ] {
            assert_eq!(scan(kind, None).request().unwrap().kind, kind);
        }
        for args in [
            &["--kind", "container", "--model", "mysql_db"][..],
            &["--kind", "container", "--repo", "mysql"],
            &["--model", "mysql_db", "--repo", "mysql"],
        ] {
            let argv = ["sherpa", "server", "image", "scan"].iter().chain(args);
            assert!(Cli::try_parse_from(argv).is_err(), "{args:?}");
        }
    }
}
