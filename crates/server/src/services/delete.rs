use anyhow::{Context, Result};
use db::ImageInUse;
use tracing::instrument;

use crate::services::image_lifecycle::{
    ensure_image_has_no_overlays, image_model_kind, lock_image,
};
use crate::services::image_store::ImagePaths;
use shared::data::{DeleteImageRequest, DeleteImageResponse, NodeKind};
use shared::util::image_filename;
use std::fs;
use std::future::Future;
use std::io::ErrorKind;
use std::path::Path;

use crate::daemon::state::AppState;
use crate::services::error::ServiceError;
use crate::validate::validate_image_version;

/// Delete an imported image: its record, then its file, or for a container its Docker image.
/// A VM and a unikernel of one model can share a version folder, so only the image's own file
/// is deleted, and the folder only once it is empty.
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
        return Err(ServiceError::NotFound(format!(
            "Image not found for model '{}' with version '{}'. Use 'server image list' to see available images.",
            request.model, request.version
        ))
        .into());
    };

    let version_dir = ImagePaths::installed().version_dir(request.model, &request.version);
    let image_file =
        image_filename(&node_image.settings).map(|filename| version_dir.join(filename));
    if kind != NodeKind::Container {
        ensure_image_has_no_overlays(state.qemu.as_ref(), &version_dir).map_err(|error| {
            match ServiceError::find(&error) {
                Some(ServiceError::Conflict(_)) => {
                    ServiceError::Conflict(format!("Cannot safely delete image: {error}")).into()
                }
                _ => error.context("Cannot safely delete image"),
            }
        })?;
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
        image_file.as_deref(),
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

/// Delete the row, then the image's file. A database failure short-circuits before any file is
/// touched. Returns what the row delete returned, and whether the file is gone.
#[instrument(level = "debug", skip(database_delete))]
async fn delete_files_after_database<T>(
    database_delete: impl Future<Output = Result<T>>,
    image_file: Option<&Path>,
) -> Result<(T, bool)> {
    let deleted = database_delete.await.map_err(|error| {
        let context = "Failed to delete image from database; image files retained";
        match error.downcast_ref::<ImageInUse>() {
            Some(in_use) => ServiceError::Conflict(format!("{context}: {in_use}")).into(),
            None => error.context(context),
        }
    })?;
    let Some(image_file) = image_file else {
        return Ok((deleted, false));
    };
    Ok((deleted, remove_image_file(image_file)))
}

/// Remove an image's file, then its version folder if that leaves it empty: another kind's file
/// at the same version keeps the folder. A file already gone counts as removed. Returns whether
/// the file is gone; a failure is logged, the row being deleted already.
fn remove_image_file(image_file: &Path) -> bool {
    match fs::remove_file(image_file) {
        Ok(()) => tracing::debug!(path = %image_file.display(), "Image file deleted"),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            tracing::error!(path = %image_file.display(), %error, "Failed to delete image file");
            return false;
        }
    }
    let Some(version_dir) = image_file.parent() else {
        return true;
    };
    match fs::remove_dir(version_dir) {
        Ok(()) => tracing::debug!(path = %version_dir.display(), "Empty version folder removed"),
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty
            ) => {}
        Err(error) => {
            tracing::warn!(path = %version_dir.display(), %error, "Failed to remove the version folder");
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::{DefaultRequest, delete_node_image, find_node_image_by_model_kind_version};
    use shared::data::{ModelKind, NodeModel};
    use shared::konst::{UNIKERNEL_KERNEL_FILENAME, VM_DISK_FILENAME};
    use std::path::PathBuf;
    use tempfile::tempdir;

    /// A `generic` VM and a `generic` unikernel imported at one version share its folder.
    fn shared_version(root: &Path) -> PathBuf {
        let version = root.join("generic").join("1.0");
        fs::create_dir_all(&version).unwrap();
        fs::write(version.join(VM_DISK_FILENAME), "vm").unwrap();
        fs::write(version.join(UNIKERNEL_KERNEL_FILENAME), "unikernel").unwrap();
        version
    }

    #[tokio::test]
    async fn image_delete_013_a_delete_whose_record_cannot_be_removed_keeps_the_images_file() {
        let root = tempdir().unwrap();
        let version = shared_version(root.path());
        let image = version.join(VM_DISK_FILENAME);
        let result = delete_files_after_database::<()>(
            async { anyhow::bail!("referenced image") },
            Some(&image),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(image).unwrap(), "vm");
    }

    #[tokio::test]
    async fn image_delete_005_deleting_one_kind_keeps_the_other_kinds_record_and_file() {
        for (deleted, kept) in [
            (NodeKind::VirtualMachine, NodeKind::Unikernel),
            (NodeKind::Unikernel, NodeKind::VirtualMachine),
        ] {
            let db = db::open_in_memory().await.unwrap();
            db::apply_schema(&db).await.unwrap();
            for kind in [deleted, kept] {
                let mut image = ModelKind::new(NodeModel::Generic, Some(kind))
                    .unwrap()
                    .node_config();
                "1.0".clone_into(&mut image.version);
                db::upsert_node_image(&db, image, DefaultRequest::Keep)
                    .await
                    .unwrap();
            }
            let id =
                find_node_image_by_model_kind_version(&db, &NodeModel::Generic, &deleted, "1.0")
                    .await
                    .unwrap()
                    .unwrap()
                    .id
                    .unwrap();
            let root = tempdir().unwrap();
            let version = shared_version(root.path());
            let file = |kind| match kind {
                NodeKind::Unikernel => version.join(UNIKERNEL_KERNEL_FILENAME),
                _ => version.join(VM_DISK_FILENAME),
            };

            let (_, image_deleted) =
                delete_files_after_database(delete_node_image(&db, id), Some(&file(deleted)))
                    .await
                    .unwrap();

            assert!(image_deleted, "{deleted}");
            assert!(!file(deleted).exists(), "{deleted}");
            assert!(file(kept).is_file(), "{deleted} took the {kept} file");
            let deleted_record =
                find_node_image_by_model_kind_version(&db, &NodeModel::Generic, &deleted, "1.0")
                    .await
                    .unwrap();
            let kept_record =
                find_node_image_by_model_kind_version(&db, &NodeModel::Generic, &kept, "1.0")
                    .await
                    .unwrap();
            assert!(deleted_record.is_none(), "{deleted}");
            assert!(kept_record.is_some(), "{deleted} took the {kept} record");
        }
    }

    #[tokio::test]
    async fn image_delete_004_deleting_an_image_removes_its_file_and_the_version_folder_with_the_last_file()
     {
        let root = tempdir().unwrap();
        let version = shared_version(root.path());
        for file in [VM_DISK_FILENAME, UNIKERNEL_KERNEL_FILENAME] {
            assert!(version.exists(), "{file}");
            let ((), image_deleted) =
                delete_files_after_database(async { Ok(()) }, Some(&version.join(file)))
                    .await
                    .unwrap();
            assert!(image_deleted, "{file}");
        }
        assert!(!version.exists());
        assert!(root.path().join("generic").is_dir());
    }

    #[tokio::test]
    async fn image_delete_014_an_image_whose_file_is_already_gone_is_deleted_with_its_empty_folder()
    {
        let root = tempdir().unwrap();
        let version = root.path().join("generic").join("1.0");
        fs::create_dir_all(&version).unwrap();
        let ((), image_deleted) =
            delete_files_after_database(async { Ok(()) }, Some(&version.join(VM_DISK_FILENAME)))
                .await
                .unwrap();
        assert!(image_deleted);
        assert!(!version.exists());
    }
}
