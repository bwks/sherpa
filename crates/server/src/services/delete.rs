use anyhow::{Context, Result};
use tracing::instrument;

use shared::data::{DeleteImageRequest, DeleteImageResponse, NodeConfig, NodeKind};
use shared::konst::SHERPA_IMAGES_PATH;
use shared::util::delete_dirs;

use crate::daemon::state::AppState;

/// Delete an imported image from both disk and database
#[instrument(skip(state), fields(model = %request.model, version = %request.version))]
pub async fn delete_image(
    request: DeleteImageRequest,
    state: &AppState,
) -> Result<DeleteImageResponse> {
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

    let node_image = match node_image {
        Some(img) => img,
        None => {
            anyhow::bail!(
                "Image not found for model '{}' with version '{}'. Use 'server image list' to see available images.",
                request.model,
                request.version
            );
        }
    };

    // Delete the database record first (DB enforces referential integrity —
    // rejects if nodes reference the image)
    let db_deleted = match &node_image.id {
        Some(id) => match db::delete_node_image(&state.db, id.clone()).await {
            Ok(()) => {
                tracing::info!(
                    "Deleted node_image record for model={} version={}",
                    request.model,
                    request.version
                );
                true
            }
            Err(e) => {
                anyhow::bail!(
                    "Failed to delete image from database (it may be referenced by active nodes): {}",
                    e
                );
            }
        },
        None => {
            tracing::warn!(
                "Image record has no ID, skipping DB delete for model={} version={}",
                request.model,
                request.version
            );
            false
        }
    };

    // Delete the version directory from disk
    let version_dir = format!(
        "{}/{}/{}",
        SHERPA_IMAGES_PATH, request.model, request.version
    );
    let disk_deleted = if kind != NodeKind::Container {
        match delete_dirs(&version_dir) {
            Ok(()) => {
                tracing::info!("Deleted image directory: {}", version_dir);
                true
            }
            Err(e) => {
                tracing::error!("Failed to delete image directory {}: {:?}", version_dir, e);
                false
            }
        }
    } else {
        // Container images live in Docker, not on disk
        tracing::info!(
            "Skipping disk delete for container image model={} version={}",
            request.model,
            request.version
        );
        false
    };

    Ok(DeleteImageResponse {
        success: true,
        model: request.model,
        kind,
        version: request.version,
        disk_deleted,
        db_deleted,
    })
}
