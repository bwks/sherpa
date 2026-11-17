use anyhow::{Context, Result};
use tracing::instrument;

use crate::services::image_lifecycle::{
    ensure_image_has_no_overlays, image_model_kind, lock_image,
};
use shared::data::{DeleteImageRequest, DeleteImageResponse, NodeKind};
use shared::konst::SHERPA_IMAGES_PATH;
use shared::util::delete_dirs;
use std::future::Future;
use std::path::Path;

use crate::daemon::state::AppState;
use crate::services::error::ServiceError;
use crate::validate::validate_image_version;

/// Delete an imported image: its record, then its files, or for a container its Docker image.
///
/// A VM or unikernel image is refused while a disk of the storage pool, running or stopped, is
/// built on it; a container image while a running container runs it. Any image a node records
/// is refused by the database.
#[instrument(skip(state), fields(model = %request.model, version = %request.version))]
pub async fn delete_image(
    request: DeleteImageRequest,
    state: &AppState,
) -> Result<DeleteImageResponse> {
    validate_image_version(&request.version)?;
    let model_kind = image_model_kind(request.model, request.kind)?;
    let _image_guard = lock_image(model_kind, &request.version).await;
    let kind = model_kind.kind();

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
        ensure_image_has_no_overlays(state.qemu.as_ref(), Path::new(&version_dir))
            .context("Cannot safely delete image")?;
    }
    // The Docker image Sherpa recorded for a container image: its repo at its version.
    let docker_reference = node_image
        .container_repo(None)
        .map(|repo| container::image_reference(&repo, &request.version));
    if let Some(reference) = &docker_reference {
        let running = container::running_containers_of_image(&state.docker, reference)
            .await
            .context("Cannot safely delete image")?;
        if !running.is_empty() {
            return Err(ServiceError::Conflict(format!(
                "Container image {reference} is in use by running container(s): {}; stop or \
                 destroy them first",
                running.join(", ")
            ))
            .into());
        }
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
    let image_deleted = match &docker_reference {
        Some(reference) => match container::remove_image(&state.docker, reference).await {
            Ok(removed) => removed,
            Err(error) => {
                tracing::error!(%reference, error = %format!("{error:#}"), "Failed to remove the Docker image");
                false
            }
        },
        None => disk_deleted,
    };

    Ok(DeleteImageResponse {
        success: true,
        model: request.model,
        kind,
        version: request.version,
        image_deleted,
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
