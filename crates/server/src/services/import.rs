use anyhow::{Context, Result};

use shared::data::{
    ImageSummary, ImportRequest, ImportResponse, ListImagesRequest, ListImagesResponse, NodeConfig,
    NodeKind,
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
        })
        .collect();

    let total = images.len();
    Ok(ListImagesResponse { images, total })
}
