use anyhow::{Context, Result};
use tracing::instrument;

use crate::services::image_lifecycle::lock_image;
use shared::data::{DeleteImageRequest, DeleteImageResponse, NodeConfig, NodeKind};
use shared::konst::{SHERPA_IMAGES_PATH, SHERPA_STORAGE_POOL_PATH};
use shared::util::delete_dirs;
use std::future::Future;
use std::path::Path;

use crate::daemon::state::AppState;
use crate::validate::validate_image_version;

/// Delete an imported image from both disk and database
#[instrument(skip(state), fields(model = %request.model, version = %request.version))]
pub async fn delete_image(
    request: DeleteImageRequest,
    state: &AppState,
) -> Result<DeleteImageResponse> {
    validate_image_version(&request.version)?;
    let _image_guard = lock_image(request.model, &request.version).await;
    let config = NodeConfig::get_model(request.model);
    let kind = config.kind.clone();

    // Look up the image in the database
    let node_image = db::find_node_image_by_model_kind_version(
        &state.db,
        &request.model,
        &kind,
        &request.version,
    )
    .await
    .context("Failed to look up image in database")?;

    let Some(node_image) = node_image else {
        anyhow::bail!(
            "Image not found for model '{}' with version '{}'. Use 'server image list' to see available images.",
            request.model,
            request.version
        );
    };

    let version_dir = format!(
        "{}/{}/{}",
        SHERPA_IMAGES_PATH, request.model, request.version
    );
    if kind != NodeKind::Container {
        libvirt::ensure_no_overlay_dependencies(
            Path::new(SHERPA_STORAGE_POOL_PATH),
            Path::new(&version_dir),
        )
        .context("Cannot safely delete image")?;
    }

    let image_id = node_image
        .id
        .context("Image record has no ID; refusing filesystem deletion")?;
    let (promoted, disk_deleted) = delete_files_after_database(
        db::delete_node_image(&state.db, image_id),
        (kind != NodeKind::Container).then_some(version_dir.as_str()),
    )
    .await?;
    if let Some(promoted) = &promoted {
        tracing::info!(model = %request.model, version = %promoted.version, "Made the newest remaining version the default");
    }

    Ok(DeleteImageResponse {
        success: true,
        model: request.model,
        kind,
        version: request.version,
        disk_deleted,
        db_deleted: true,
        new_default: promoted.map(|image| image.version),
    })
}

/// Delete the row, then the image files. A database failure short-circuits before any file is
/// touched. Returns what the row delete returned, and whether the files were deleted.
#[instrument(level = "debug", skip(database_delete))]
async fn delete_files_after_database<T>(
    database_delete: impl Future<Output = Result<T>>,
    version_dir: Option<&str>,
) -> Result<(T, bool)> {
    let deleted = database_delete
        .await
        .context("Failed to delete image from database; image files retained")?;
    let Some(version_dir) = version_dir else {
        return Ok((deleted, false));
    };
    match delete_dirs(version_dir) {
        Ok(()) => Ok((deleted, true)),
        Err(error) => {
            tracing::error!(%version_dir, %error, "Failed to delete image directory");
            Ok((deleted, false))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn rejected_database_deletion_preserves_image_files() {
        let root = tempdir().unwrap();
        let image = root.path().join("disk.qcow2");
        fs::write(&image, "immutable image").unwrap();
        let result = delete_files_after_database::<()>(
            async { anyhow::bail!("referenced image") },
            Some(root.path().to_str().unwrap()),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(image).unwrap(), "immutable image");
    }

    #[tokio::test]
    async fn accepted_database_deletion_removes_image_files() {
        let root = tempdir().unwrap();
        let directory = root.path().join("version");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("disk.qcow2"), "image").unwrap();
        let ((), disk_deleted) =
            delete_files_after_database(async { Ok(()) }, Some(directory.to_str().unwrap()))
                .await
                .unwrap();
        assert!(disk_deleted);
        assert!(!directory.exists());
    }
}
