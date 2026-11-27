//! The base image sandboxes boot: the default `flatcar_sandbox` image, Flatcar's `OpenStack` image
//! as the user imported it.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::instrument;

use db::Database;
use shared::data::{ModelKind, NodeKind, NodeModel};
use shared::konst::VM_DISK_FILENAME;

use crate::services::error::ServiceError;
use crate::services::image_lifecycle::{ImageLease, lease_image};
use crate::services::image_store::ImagePaths;

/// The version and disk of the base image sandboxes boot, with a lease that keeps it from being
/// replaced or deleted while disks are made on it.
#[derive(Clone)]
pub(crate) struct SandboxBase {
    pub(crate) version: String,
    pub(crate) disk: String,
    pub(crate) _lease: ImageLease,
}

/// The base image when one of `model_kinds` runs in a sandbox: the disk of the default
/// `flatcar_sandbox` image in images folder `images_dir`, leased.
///
/// # Errors
///
/// Fails with [`ServiceError::SandboxBaseImageMissing`] when the server has no default
/// `flatcar_sandbox` image, with [`ServiceError::SandboxBaseDiskMissing`] when its disk is not
/// there, and when the database cannot be read.
#[instrument(skip(db, model_kinds), level = "debug")]
pub(crate) async fn sandbox_base(
    db: &Database,
    images_dir: &str,
    model_kinds: &[ModelKind],
) -> Result<Option<SandboxBase>> {
    if !model_kinds.iter().any(|kind| kind.runs_in_sandbox()) {
        return Ok(None);
    }
    let image =
        db::find_default_node_image(db, &NodeModel::FlatcarSandbox, &NodeKind::VirtualMachine)
            .await?
            .ok_or(ServiceError::SandboxBaseImageMissing)?;
    let lease = lease_image(image.model_kind()?, &image.version).await;
    let disk = base_disk(images_dir, &image.version);
    if !disk.is_file() {
        return Err(ServiceError::SandboxBaseDiskMissing {
            version: image.version,
            path: disk,
        }
        .into());
    }
    Ok(Some(SandboxBase {
        version: image.version,
        disk: disk
            .to_str()
            .with_context(|| {
                format!(
                    "The sandbox base image path {} is not UTF-8",
                    disk.display()
                )
            })?
            .to_owned(),
        _lease: lease,
    }))
}

/// The disk of version `version` of the `flatcar_sandbox` image in images folder `images_dir`.
fn base_disk(images_dir: &str, version: &str) -> PathBuf {
    ImagePaths {
        images: Path::new(images_dir).to_owned(),
    }
    .version_dir(NodeModel::FlatcarSandbox, version)
    .join(VM_DISK_FILENAME)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use db::DefaultRequest;
    use shared::data::NodeKind;

    use super::*;

    #[tokio::test]
    async fn sandbox_018_a_lab_with_a_sandbox_is_refused_without_the_flatcar_sandbox_image() {
        let dir = TempDir::new().unwrap();
        let images = dir.path().to_str().unwrap();
        let database = db::open_in_memory().await.unwrap();
        db::apply_schema(&database).await.unwrap();
        let ceos = ModelKind::new(NodeModel::AristaEos, Some(NodeKind::Container)).unwrap();
        let frr = ModelKind::new(NodeModel::FrrLinux, Some(NodeKind::Container)).unwrap();
        assert!(
            sandbox_base(&database, images, &[frr])
                .await
                .unwrap()
                .is_none()
        );
        let missing = sandbox_base(&database, images, &[frr, ceos])
            .await
            .err()
            .unwrap();
        assert!(matches!(
            missing.downcast_ref::<ServiceError>(),
            Some(ServiceError::SandboxBaseImageMissing)
        ));
        assert!(
            missing
                .to_string()
                .contains("sherpa server image import --src flatcar_production_openstack_image.img --model flatcar_sandbox")
        );

        let mut image = ModelKind::default_for(NodeModel::FlatcarSandbox).node_config();
        image.version = "4757.2.4".to_owned();
        db::upsert_node_image(&database, image, DefaultRequest::Make)
            .await
            .unwrap();
        let no_disk = sandbox_base(&database, images, &[ceos])
            .await
            .err()
            .unwrap();
        assert!(matches!(
            no_disk.downcast_ref::<ServiceError>(),
            Some(ServiceError::SandboxBaseDiskMissing { version, .. }) if version == "4757.2.4"
        ));

        let disk = base_disk(images, "4757.2.4");
        fs::create_dir_all(disk.parent().unwrap()).unwrap();
        fs::write(&disk, b"qcow2").unwrap();
        let base = sandbox_base(&database, images, &[frr, ceos])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(base.version, "4757.2.4");
        assert_eq!(
            base.disk,
            format!("{images}/flatcar_sandbox/4757.2.4/{VM_DISK_FILENAME}")
        );
    }
}
