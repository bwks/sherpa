use anyhow::{Context, Result};

use shared::data::{
    ImageSummary, ImportRequest, ImportResponse, ListImagesRequest, ListImagesResponse, NodeConfig,
    NodeKind, NodeModel, ScanImagesRequest, ScanImagesResponse, ScannedImage,
};
use shared::konst::{SHERPA_BASE_DIR, SHERPA_IMAGES_DIR};
use shared::util::{copy_file, create_dir, create_symlink, file_exists, fix_permissions_recursive};

use crate::daemon::state::AppState;

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

    let images_dir = format!("{SHERPA_BASE_DIR}/{SHERPA_IMAGES_DIR}");
    let model_dir = format!("{images_dir}/{}", request.model);
    let version_dir = format!("{model_dir}/{}", request.version);
    let version_disk = format!("{version_dir}/virtioa.qcow2");

    // Create version directory
    create_dir(&version_dir).context("Failed to create version directory")?;

    // Copy image file if it doesn't already exist
    if !file_exists(&version_disk) {
        tracing::info!("Copying image from {} to {}", request.src, version_disk);
        copy_file(&request.src, &version_disk).context("Failed to copy image file")?;
    } else {
        tracing::info!("Image already exists at {}, skipping copy", version_disk);
    }

    // Create latest symlink if requested
    if request.latest {
        let latest_dir = format!("{model_dir}/latest");
        create_dir(&latest_dir).context("Failed to create latest directory")?;
        let latest_disk = format!("{latest_dir}/virtioa.qcow2");
        tracing::info!("Creating symlink from {} to {}", version_disk, latest_disk);
        create_symlink(&version_disk, &latest_disk).context("Failed to create latest symlink")?;
    }

    // Fix permissions on the images directory
    fix_permissions_recursive(&images_dir).context("Failed to set image permissions")?;

    // Upsert node_config record in the database
    let mut db_config = config;
    db_config.version = request.version.clone();
    db_config.default = request.latest;
    db_config.active = true;
    db_config.id = None;

    let db_tracked = match db::upsert_node_config(&state.db, db_config).await {
        Ok(_) => {
            tracing::info!(
                "Upserted node_config for model={} version={}",
                request.model,
                request.version
            );
            true
        }
        Err(e) => {
            tracing::error!(
                "Failed to upsert node_config for model={} version={}: {:?}",
                request.model,
                request.version,
                e
            );
            false
        }
    };

    Ok(ImportResponse {
        success: true,
        model: request.model,
        kind,
        version: request.version,
        image_path: version_disk,
        db_tracked,
    })
}

/// List images from the database with optional filtering by model and/or kind
pub async fn list_images(
    request: ListImagesRequest,
    state: &AppState,
) -> Result<ListImagesResponse> {
    let configs = match (&request.model, &request.kind) {
        (Some(model), Some(kind)) => db::get_node_config_versions(&state.db, model, kind)
            .await
            .context("Failed to query node configs by model and kind")?,
        (None, Some(kind)) => db::list_node_configs_by_kind(&state.db, kind)
            .await
            .context("Failed to query node configs by kind")?,
        (Some(model), None) => {
            let config = NodeConfig::get_model(*model);
            db::get_node_config_versions(&state.db, model, &config.kind)
                .await
                .context("Failed to query node configs by model")?
        }
        (None, None) => db::list_node_configs(&state.db)
            .await
            .context("Failed to query all node configs")?,
    };

    let images: Vec<ImageSummary> = configs
        .into_iter()
        .map(|c| ImageSummary {
            model: c.model,
            kind: c.kind,
            version: c.version,
            default: c.default,
            active: c.active,
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
    let images_dir = format!("{SHERPA_BASE_DIR}/{SHERPA_IMAGES_DIR}");
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

        // Skip non-VM kinds (for now)
        if kind != NodeKind::VirtualMachine {
            tracing::debug!("Skipping non-VM model: {} (kind={})", model, kind);
            continue;
        }

        // If a kind filter was provided, check it
        if let Some(ref filter_kind) = request.kind
            && *filter_kind != kind
        {
            continue;
        }

        // Check if the only existing record is the seeded "inactive" placeholder.
        // If so, the first imported version should become the default.
        let existing_versions = db::get_node_config_versions(&state.db, &model, &kind)
            .await
            .context("Failed to query existing node_config versions")?;
        let only_inactive = existing_versions.len() <= 1
            && existing_versions
                .first()
                .is_none_or(|c| c.version == "inactive" && !c.active);
        let mut set_default = only_inactive;

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

            // Check if record already exists and is active
            let existing =
                db::get_node_config_by_model_kind_version(&state.db, &model, &kind, &version)
                    .await
                    .context("Failed to check existing node_config")?;

            if let Some(ref existing_config) = existing
                && existing_config.active
            {
                scanned.push(ScannedImage {
                    model,
                    version,
                    kind: kind.clone(),
                    status: "already_active".to_string(),
                });
                continue;
            }

            // First import for a model with only the inactive placeholder gets default
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

            // Upsert with active: true
            let mut db_config = config.clone();
            db_config.version = version.clone();
            db_config.default = make_default;
            db_config.active = true;
            db_config.id = None;

            let status = match db::upsert_node_config(&state.db, db_config).await {
                Ok(_) => {
                    tracing::info!(
                        "Scanned and imported node_config for model={} version={} default={}",
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
                        "Failed to upsert node_config for model={} version={}: {:?}",
                        model,
                        version,
                        e
                    );
                    format!("error: {}", e)
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

    Ok(ScanImagesResponse {
        scanned,
        total_imported,
    })
}
