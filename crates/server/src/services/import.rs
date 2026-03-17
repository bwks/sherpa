use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;

use shared::data::{
    DownloadImageRequest, ImageSummary, ImportRequest, ImportResponse, ListImagesRequest,
    ListImagesResponse, NodeConfig, NodeKind, NodeModel, ScanImagesRequest, ScanImagesResponse,
    ScannedImage, SetDefaultImageRequest, SetDefaultImageResponse, StatusKind,
};
use shared::konst::SHERPA_IMAGES_PATH;
use shared::util::{copy_file, create_dir, file_exists};

use crate::daemon::state::AppState;
use crate::services::progress::ProgressSender;

/// Import a disk image to the server filesystem and track it in the database
pub async fn import_image(request: ImportRequest, state: &AppState) -> Result<ImportResponse> {
    let config = NodeConfig::get_model(request.model);
    let kind = config.kind.clone();

    // Only VM and Unikernel imports are supported currently
    if kind == NodeKind::Container {
        anyhow::bail!("Container image import is not yet implemented");
    }

    // Validate source file exists on the server
    if !file_exists(&request.src) {
        anyhow::bail!("Source file does not exist: {}", request.src);
    }

    // If this is the first image for this model+kind, mark it as default
    let existing_versions = db::get_node_image_versions(&state.db, &request.model, &kind).await?;
    let make_default = if existing_versions.is_empty() {
        true
    } else {
        request.default
    };

    // Validate node model is accepted by the database before copying files
    let mut db_config = config;
    db_config.version = request.version.clone();
    db_config.default = make_default;
    db_config.id = None;

    db::upsert_node_image(&state.db, db_config)
        .await
        .context(format!(
            "Failed to register node model '{}' in database. Ensure the server is up to date.",
            request.model
        ))?;

    tracing::info!(
        "Upserted node_image for model={} version={}",
        request.model,
        request.version
    );

    // Copy image file
    let images_dir = SHERPA_IMAGES_PATH.to_owned();
    let model_dir = format!("{images_dir}/{}", request.model);
    let version_dir = format!("{model_dir}/{}", request.version);
    let version_disk = format!("{version_dir}/virtioa.qcow2");

    create_dir(&version_dir).context("Failed to create version directory")?;

    if !file_exists(&version_disk) {
        tracing::info!("Copying image from {} to {}", request.src, version_disk);
        copy_file(&request.src, &version_disk).context("Failed to copy image file")?;
    } else {
        tracing::info!("Image already exists at {}, skipping copy", version_disk);
    }

    Ok(ImportResponse {
        success: true,
        model: request.model,
        kind,
        version: request.version,
        image_path: version_disk,
        db_tracked: true,
    })
}

/// List images from the database with optional filtering by model and/or kind
pub async fn list_images(
    request: ListImagesRequest,
    state: &AppState,
) -> Result<ListImagesResponse> {
    let configs = match (&request.model, &request.kind) {
        (Some(model), Some(kind)) => db::get_node_image_versions(&state.db, model, kind)
            .await
            .context("Failed to query node images by model and kind")?,
        (None, Some(kind)) => db::list_node_images_by_kind(&state.db, kind)
            .await
            .context("Failed to query node images by kind")?,
        (Some(model), None) => {
            let config = NodeConfig::get_model(*model);
            db::get_node_image_versions(&state.db, model, &config.kind)
                .await
                .context("Failed to query node images by model")?
        }
        (None, None) => db::list_node_images(&state.db)
            .await
            .context("Failed to query all node images")?,
    };

    let images: Vec<ImageSummary> = configs
        .into_iter()
        .map(|c| ImageSummary {
            model: c.model,
            kind: c.kind,
            version: c.version,
            default: c.default,
        })
        .collect();

    let total = images.len();
    Ok(ListImagesResponse { images, total })
}

/// Scan the images directory for on-disk VM images and import them into the database
pub async fn scan_images(
    request: ScanImagesRequest,
    state: &AppState,
) -> Result<ScanImagesResponse> {
    let images_dir = SHERPA_IMAGES_PATH.to_owned();
    let mut scanned: Vec<ScannedImage> = Vec::new();
    let mut total_imported: usize = 0;

    // Read model directories from the images dir
    let model_entries = tokio::fs::read_dir(&images_dir)
        .await
        .context(format!("Failed to read images directory: {}", images_dir))?;

    let mut model_entries = model_entries;
    while let Some(entry) = model_entries
        .next_entry()
        .await
        .context("Failed to read directory entry")?
    {
        // Skip non-directories
        let metadata = match entry.metadata().await {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !metadata.is_dir() {
            continue;
        }

        let dir_name = entry.file_name().to_string_lossy().to_string();

        // Try to parse the directory name as a NodeModel
        let model: NodeModel = match dir_name.parse() {
            Ok(m) => m,
            Err(_) => {
                tracing::debug!("Skipping non-model directory: {}", dir_name);
                continue;
            }
        };

        // Get the model's config to determine kind
        let config = NodeConfig::get_model(model);
        let kind = config.kind.clone();

        // Skip container kinds in filesystem scan (containers live in Docker, not on disk)
        if kind == NodeKind::Container {
            tracing::debug!("Skipping container model in disk scan: {}", model);
            continue;
        }

        // If a kind filter was provided, check it
        if let Some(ref filter_kind) = request.kind
            && *filter_kind != kind
        {
            continue;
        }

        // Check if any existing records exist for this model.
        // If none exist, the first imported version should become the default.
        let existing_versions = db::get_node_image_versions(&state.db, &model, &kind)
            .await
            .context("Failed to query existing node_image versions")?;
        let mut set_default = existing_versions.is_empty();

        // Iterate version subdirectories
        let model_dir = format!("{}/{}", images_dir, dir_name);
        let mut version_entries = match tokio::fs::read_dir(&model_dir).await {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!("Failed to read model directory {}: {}", model_dir, e);
                continue;
            }
        };

        while let Some(version_entry) = version_entries
            .next_entry()
            .await
            .context("Failed to read version directory entry")?
        {
            let version_metadata = match version_entry.metadata().await {
                Ok(m) => m,
                Err(_) => continue,
            };

            // Skip symlinks and non-directories
            if !version_metadata.is_dir() || version_metadata.file_type().is_symlink() {
                continue;
            }

            // Also check via symlink_metadata to catch symlinked directories
            let symlink_meta = match tokio::fs::symlink_metadata(version_entry.path()).await {
                Ok(m) => m,
                Err(_) => continue,
            };
            if symlink_meta.file_type().is_symlink() {
                continue;
            }

            let version = version_entry.file_name().to_string_lossy().to_string();
            let disk_path = format!("{}/{}/virtioa.qcow2", model_dir, version);

            // Check if virtioa.qcow2 exists in this version directory
            if !file_exists(&disk_path) {
                tracing::debug!(
                    "No virtioa.qcow2 found for model={} version={}",
                    model,
                    version
                );
                continue;
            }

            // Check if record already exists with this version
            let existing =
                db::get_node_image_by_model_kind_version(&state.db, &model, &kind, &version)
                    .await
                    .context("Failed to check existing node_image")?;

            if existing.is_some() {
                scanned.push(ScannedImage {
                    model,
                    version,
                    kind: kind.clone(),
                    status: "already_exists".to_string(),
                });
                continue;
            }

            // First import for a model with no existing records gets default
            let make_default = set_default;

            if request.dry_run {
                let status = if make_default {
                    "would_import (default)"
                } else {
                    "would_import"
                };
                scanned.push(ScannedImage {
                    model,
                    version,
                    kind: kind.clone(),
                    status: status.to_string(),
                });
                total_imported += 1;
                // Only the first version gets default
                set_default = false;
                continue;
            }

            let mut db_config = config.clone();
            db_config.version = version.clone();
            db_config.default = make_default;
            db_config.id = None;

            let status = match db::upsert_node_image(&state.db, db_config).await {
                Ok(_) => {
                    tracing::info!(
                        "Scanned and imported node_image for model={} version={} default={}",
                        model,
                        version,
                        make_default
                    );
                    total_imported += 1;
                    if make_default {
                        "imported (default)".to_string()
                    } else {
                        "imported".to_string()
                    }
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to upsert node_image for model={} version={}: {:?}",
                        model,
                        version,
                        e
                    );
                    format!("error: {:#}", e)
                }
            };

            // Only the first version gets default
            set_default = false;

            scanned.push(ScannedImage {
                model,
                version,
                kind: kind.clone(),
                status,
            });
        }
    }

    // --- Container image scanning ---
    // Query Docker for local images and match against known container models
    let skip_containers = matches!(request.kind, Some(ref k) if *k != NodeKind::Container);
    if !skip_containers {
        let docker_tags = container::get_local_images(&state.docker)
            .await
            .context("Failed to list local Docker images")?;

        for model in NodeModel::to_vec() {
            let config = NodeConfig::get_model(model);
            let kind = config.kind.clone();

            // Only process container models
            if kind != NodeKind::Container {
                continue;
            }

            // If a kind filter was provided, check it (redundant guard for safety)
            if let Some(ref filter_kind) = request.kind
                && *filter_kind != kind
            {
                continue;
            }

            // Skip models with no repo (no way to match Docker images)
            let repo = match &config.repo {
                Some(r) => r.clone(),
                None => {
                    tracing::debug!("Skipping container model {} (no repo configured)", model);
                    continue;
                }
            };

            // Check if any existing records exist for this model
            let existing_versions = db::get_node_image_versions(&state.db, &model, &kind)
                .await
                .context("Failed to query existing node_image versions")?;
            let mut set_default = existing_versions.is_empty();

            let prefix = format!("{}:", repo);
            for tag in &docker_tags {
                // Match tags like "localrepo/arista_ceos:4.32.0F"
                let version = match tag.strip_prefix(&prefix) {
                    Some(v) => v.to_string(),
                    None => continue,
                };

                // Check if record already exists with this version
                let existing =
                    db::get_node_image_by_model_kind_version(&state.db, &model, &kind, &version)
                        .await
                        .context("Failed to check existing node_image")?;

                if existing.is_some() {
                    scanned.push(ScannedImage {
                        model,
                        version,
                        kind: kind.clone(),
                        status: "already_exists".to_string(),
                    });
                    continue;
                }

                let make_default = set_default;

                if request.dry_run {
                    let status = if make_default {
                        "would_import (default)"
                    } else {
                        "would_import"
                    };
                    scanned.push(ScannedImage {
                        model,
                        version,
                        kind: kind.clone(),
                        status: status.to_string(),
                    });
                    total_imported += 1;
                    set_default = false;
                    continue;
                }

                let mut db_config = config.clone();
                db_config.version = version.clone();
                db_config.default = make_default;
                db_config.id = None;

                let status = match db::upsert_node_image(&state.db, db_config).await {
                    Ok(_) => {
                        tracing::info!(
                            "Scanned and imported container node_image for model={} version={} default={}",
                            model,
                            version,
                            make_default
                        );
                        total_imported += 1;
                        if make_default {
                            "imported (default)".to_string()
                        } else {
                            "imported".to_string()
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to upsert node_image for model={} version={}: {:?}",
                            model,
                            version,
                            e
                        );
                        format!("error: {:#}", e)
                    }
                };

                set_default = false;

                scanned.push(ScannedImage {
                    model,
                    version,
                    kind: kind.clone(),
                    status,
                });
            }
        }
    }

    Ok(ScanImagesResponse {
        scanned,
        total_imported,
    })
}

/// Set the default version for a node image
pub async fn set_default_image(
    request: SetDefaultImageRequest,
    state: &AppState,
) -> Result<SetDefaultImageResponse> {
    let config = NodeConfig::get_model(request.model);
    let kind = config.kind.clone();

    // Look up the image in the database
    let node_image = db::get_node_image_by_model_kind_version(
        &state.db,
        &request.model,
        &kind,
        &request.version,
    )
    .await
    .context("Failed to look up image in database")?;

    let mut node_image = match node_image {
        Some(img) => img,
        None => {
            anyhow::bail!(
                "Image not found for model '{}' with version '{}'. Use 'server image list' to see available images.",
                request.model,
                request.version
            );
        }
    };

    // Set as default — update_node_image already unsets other defaults
    node_image.default = true;

    db::update_node_image(&state.db, node_image)
        .await
        .context("Failed to update image as default")?;

    Ok(SetDefaultImageResponse {
        success: true,
        model: request.model,
        kind,
        version: request.version,
    })
}

/// Resolve the download URL for a VM image
fn resolve_download_url(model: &NodeModel, version: &str, url: Option<&str>) -> Result<String> {
    if let Some(url) = url {
        return Ok(url.to_string());
    }

    match model {
        NodeModel::UbuntuLinux => Ok(format!(
            "https://cloud-images.ubuntu.com/releases/{version}/release/ubuntu-{version}-server-cloudimg-amd64.img"
        )),
        _ => anyhow::bail!("No auto-download URL for model '{}'. Provide --url.", model),
    }
}

/// Download a VM image from a URL and track it in the database
pub async fn download_image(
    request: DownloadImageRequest,
    state: &AppState,
    progress: ProgressSender,
) -> Result<ImportResponse> {
    let config = NodeConfig::get_model(request.model);
    let kind = config.kind.clone();

    if kind != NodeKind::VirtualMachine {
        anyhow::bail!(
            "Image download is only supported for virtual machine models, got '{}'",
            kind
        );
    }

    let _ = progress.send_status("Resolving download URL...".to_string(), StatusKind::Info);

    let download_url =
        resolve_download_url(&request.model, &request.version, request.url.as_deref())?;

    let images_dir = SHERPA_IMAGES_PATH.to_owned();
    let model_dir = format!("{images_dir}/{}", request.model);
    let version_dir = format!("{model_dir}/{}", request.version);
    let version_disk = format!("{version_dir}/virtioa.qcow2");

    // Create version directory
    create_dir(&version_dir).context("Failed to create version directory")?;

    // Skip download if file already exists
    if file_exists(&version_disk) {
        tracing::info!(
            "Image already exists at {}, skipping download",
            version_disk
        );
        let _ = progress.send_status(
            "Image already exists, skipping download".to_string(),
            StatusKind::Info,
        );
    } else {
        tracing::info!(
            "Downloading image from {} to {}",
            download_url,
            version_disk
        );

        let _ = progress.send_status(
            format!(
                "Downloading {} {} from {}...",
                request.model, request.version, download_url
            ),
            StatusKind::Progress,
        );

        let response = reqwest::get(&download_url)
            .await
            .context(format!("Failed to download image from {}", download_url))?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Failed to download image: HTTP {} from {}",
                response.status(),
                download_url
            );
        }

        // Get content length for progress reporting
        let total_size = response.content_length();

        // Stream the response body to a file in chunks
        let mut file = tokio::fs::File::create(&version_disk)
            .await
            .context(format!("Failed to create file {}", version_disk))?;

        let mut downloaded: u64 = 0;
        let mut last_reported: u64 = 0;
        let report_interval: u64 = 5 * 1024 * 1024; // Report every 5MB

        let mut stream = response.bytes_stream();
        use futures_util::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("Failed to read response chunk")?;
            file.write_all(&chunk)
                .await
                .context("Failed to write chunk to disk")?;

            downloaded += chunk.len() as u64;

            // Report progress every ~5MB
            if downloaded - last_reported >= report_interval {
                last_reported = downloaded;
                let msg = if let Some(total) = total_size {
                    let percent = (downloaded as f64 / total as f64 * 100.0) as u64;
                    format!(
                        "Downloaded {:.1} MB / {:.1} MB ({}%)",
                        downloaded as f64 / 1_048_576.0,
                        total as f64 / 1_048_576.0,
                        percent
                    )
                } else {
                    format!("Downloaded {:.1} MB", downloaded as f64 / 1_048_576.0)
                };
                let _ = progress.send_status(msg, StatusKind::Progress);
            }
        }

        file.flush().await.context("Failed to flush image file")?;

        tracing::info!("Download complete: {}", version_disk);

        // Final download size message
        let final_msg = if let Some(total) = total_size {
            format!("Download complete: {:.1} MB", total as f64 / 1_048_576.0)
        } else {
            format!(
                "Download complete: {:.1} MB",
                downloaded as f64 / 1_048_576.0
            )
        };
        let _ = progress.send_status(final_msg, StatusKind::Done);
    }

    let _ = progress.send_status("Updating database...".to_string(), StatusKind::Progress);

    // Upsert node_image record in the database
    let mut db_config = config;
    db_config.version = request.version.clone();
    db_config.default = request.default;
    db_config.id = None;

    let db_tracked = match db::upsert_node_image(&state.db, db_config).await {
        Ok(_) => {
            tracing::info!(
                "Upserted node_image for model={} version={}",
                request.model,
                request.version
            );
            true
        }
        Err(e) => {
            tracing::error!(
                "Failed to upsert node_image for model={} version={}: {:?}",
                request.model,
                request.version,
                e
            );
            false
        }
    };

    let _ = progress.send_status("Image tracked in database".to_string(), StatusKind::Done);

    Ok(ImportResponse {
        success: true,
        model: request.model,
        kind,
        version: request.version,
        image_path: version_disk,
        db_tracked,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_download_url_ubuntu() {
        let url = resolve_download_url(&NodeModel::UbuntuLinux, "24.04", None).unwrap();
        assert_eq!(
            url,
            "https://cloud-images.ubuntu.com/releases/24.04/release/ubuntu-24.04-server-cloudimg-amd64.img"
        );
    }

    #[test]
    fn test_resolve_download_url_user_override() {
        let custom = "https://example.com/my-image.qcow2";
        let url = resolve_download_url(&NodeModel::FedoraLinux, "42", Some(custom)).unwrap();
        assert_eq!(url, custom);
    }

    #[test]
    fn test_resolve_download_url_unsupported_model_no_url() {
        let result = resolve_download_url(&NodeModel::FedoraLinux, "42", None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("No auto-download URL"));
        assert!(err.contains("fedora_linux"));
    }
}
