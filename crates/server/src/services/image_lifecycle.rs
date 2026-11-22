use std::path::Path;
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result};
use bollard::Docker;
use dashmap::DashMap;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use tracing::instrument;

use libvirt::{BackedVolume, Qemu};
use shared::data::{DiskImageCloneMethod, ModelKind, NodeConfig, NodeKind, NodeModel};
use shared::konst::VM_DISK_FILENAME;

use crate::services::error::ServiceError;

/// The volumes of the storage pool built on a backing file, each an overlay disk and its base:
/// what decides whether an image is in use by a lab disk, running or stopped.
pub(crate) trait BackedVolumes {
    /// The pool's backed volumes.
    ///
    /// # Errors
    ///
    /// Fails when they cannot be read: a dependency that cannot be determined is an error.
    fn backed_volumes(&self) -> Result<Vec<BackedVolume>>;
}

/// libvirt reads each volume's backing file, which sherpad cannot: the disks are `0600` and
/// owned by `libvirt-qemu` or `root`.
impl BackedVolumes for Qemu {
    fn backed_volumes(&self) -> Result<Vec<BackedVolume>> {
        let qemu_conn = self.connect()?;
        libvirt::pool_backed_volumes(&qemu_conn)
    }
}

/// Refuse to delete or replace the image in `version_dir` while a disk of `pool`, running or
/// stopped, is built on it. The caller holds the image's lifecycle lock through the check and
/// the change.
///
/// # Errors
///
/// Fails with [`ServiceError::Conflict`] when a disk is built on the image, naming the disk, and
/// otherwise when the pool's disks or their bases cannot be read.
#[instrument(level = "debug", skip(pool))]
pub(crate) fn ensure_image_has_no_overlays(
    pool: &impl BackedVolumes,
    version_dir: &Path,
) -> Result<()> {
    let volumes = pool.backed_volumes()?;
    match libvirt::overlay_on_image(&volumes, version_dir)? {
        Some(volume) => Err(ServiceError::Conflict(format!(
            "Disk {} is built on the image at {}; destroy or redeploy its node first",
            volume.name,
            version_dir.display()
        ))
        .into()),
        None => Ok(()),
    }
}

/// The model and kind an image request names: its kind, or the model's default kind when it
/// names none.
///
/// # Errors
///
/// Fails with [`ServiceError::Invalid`] when the model does not take the kind.
pub(crate) fn image_model_kind(model: NodeModel, kind: Option<NodeKind>) -> Result<ModelKind> {
    ModelKind::new(model, kind).map_err(|error| ServiceError::Invalid(format!("{error:#}")).into())
}

static IMAGE_LOCKS: OnceLock<DashMap<String, Arc<RwLock<()>>>> = OnceLock::new();

pub(crate) type ImageLease = Arc<OwnedRwLockReadGuard<()>>;

/// The lock key of an image: its model, kind and version.
fn image_lock_key(model_kind: ModelKind, version: &str) -> String {
    format!("{}/{}/{version}", model_kind.model(), model_kind.kind())
}

/// Shared across `AppState` instances and all image-mutating workflows in this process. An
/// image is locked by its model, kind and version.
#[instrument(level = "debug")]
pub(crate) async fn lock_image(model_kind: ModelKind, version: &str) -> OwnedRwLockWriteGuard<()> {
    let key = image_lock_key(model_kind, version);
    let lock = IMAGE_LOCKS
        .get_or_init(DashMap::new)
        .entry(key)
        .or_insert_with(|| Arc::new(RwLock::new(())))
        .clone();
    lock.write_owned().await
}

/// Allow simultaneous provisioning while excluding import and deletion. Cloned
/// leases keep protection alive until blocking disk tasks finish even if their
/// caller is cancelled.
#[instrument(level = "debug")]
pub(crate) async fn lease_image(model_kind: ModelKind, version: &str) -> ImageLease {
    let key = image_lock_key(model_kind, version);
    let lock = IMAGE_LOCKS
        .get_or_init(DashMap::new)
        .entry(key)
        .or_insert_with(|| Arc::new(RwLock::new(())))
        .clone();
    Arc::new(lock.read_owned().await)
}

/// Validate policy and base before any resource creation or redeploy teardown. A refusal
/// names the node `node_name`.
#[instrument(level = "debug", skip(image))]
pub(crate) fn validate_disk_method(
    node_name: &str,
    method: Option<DiskImageCloneMethod>,
    image: &NodeConfig,
    images_dir: &str,
    size_gb: Option<u16>,
) -> Result<()> {
    if image.kind() != NodeKind::VirtualMachine {
        if method == Some(DiskImageCloneMethod::Overlay) {
            return Err(ServiceError::Invalid(format!(
                "Disk provisioning validation failed for node {node_name}: Overlay provisioning is supported only for VM boot disks"
            ))
            .into());
        }
        return Ok(());
    }
    if method.unwrap_or_default() == DiskImageCloneMethod::Copy {
        return Ok(());
    }
    let path = Path::new(images_dir)
        .join(image.model.to_string())
        .join(&image.version)
        .join(VM_DISK_FILENAME);
    libvirt::read_qcow2_metadata(&path)
        .with_context(|| format!("Disk provisioning validation failed for node {node_name}"))?
        .overlay_capacity(size_gb)
        .context("VM boot image is ineligible for overlay provisioning")
        .map_err(|error| {
            ServiceError::Invalid(format!(
                "Disk provisioning validation failed for node {node_name}: {error:#}"
            ))
        })?;
    Ok(())
}

/// Check that Docker has the image container node `node_name` is created from: `repo` at
/// `version`, the reference the image was pulled as and the container is created with. Docker
/// resolves the reference, so an official Docker Hub image is found by its full name
/// (`docker.io/library/nats:2.11`) or its short one (`nats:2.11`). Run before any resource
/// creation or redeploy teardown.
#[instrument(level = "debug", skip(docker))]
pub(crate) async fn check_container_image(
    docker: &Docker,
    node_name: &str,
    model: NodeModel,
    repo: &str,
    version: &str,
) -> Result<()> {
    let reference = container::image_reference(repo, version);
    let found = container::image_exists(docker, &reference)
        .await
        .with_context(|| format!("Cannot check the container image of node '{node_name}'"))?;
    if !found {
        return Err(ServiceError::Conflict(format!(
            "Container image not found in local Docker for node '{node_name}': {reference}\n\
             Hint: Pull the image with: sherpa server image pull --model {model} --repo {reference}"
        ))
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use tempfile::tempdir;

    #[tokio::test]
    async fn image_delete_017_operations_on_one_image_version_run_one_at_a_time_and_another_version_is_not_held_up()
     {
        let held = lock_image(
            ModelKind::default_for(NodeModel::UbuntuLinux),
            "lock-test-one",
        )
        .await;
        let waiting = tokio::spawn(lock_image(
            ModelKind::default_for(NodeModel::UbuntuLinux),
            "lock-test-one",
        ));
        let other = lock_image(
            ModelKind::default_for(NodeModel::UbuntuLinux),
            "lock-test-two",
        )
        .await;
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        drop(held);
        let acquired = waiting.await.unwrap();
        drop(acquired);
        drop(other);
    }

    #[test]
    fn unsupported_kinds_do_not_fall_back_to_copy() {
        for model in [NodeModel::MysqlDb, NodeModel::NanosUnikernel] {
            let image = ModelKind::default_for(model).node_config();
            assert!(validate_disk_method("n1", None, &image, "/missing", None).is_ok());
            assert!(
                validate_disk_method(
                    "n1",
                    Some(DiskImageCloneMethod::Overlay),
                    &image,
                    "/missing",
                    None
                )
                .is_err()
            );
            assert!(
                validate_disk_method(
                    "n1",
                    Some(DiskImageCloneMethod::Copy),
                    &image,
                    "/missing",
                    None
                )
                .is_ok()
            );
        }
    }
    #[test]
    fn omitted_vm_method_validates_the_base_as_an_overlay() {
        let root = tempdir().unwrap();
        let image = ModelKind::default_for(NodeModel::UbuntuLinux).node_config();
        let directory = root
            .path()
            .join(image.model.to_string())
            .join(&image.version);
        fs::create_dir_all(&directory).unwrap();
        let base = directory.join(VM_DISK_FILENAME);
        fs::write(&base, "not a QCOW2 image").unwrap();
        let images_dir = root.path().to_str().unwrap();
        assert!(validate_disk_method("n1", None, &image, images_dir, None).is_err());
        assert!(
            validate_disk_method(
                "n1",
                Some(DiskImageCloneMethod::Copy),
                &image,
                images_dir,
                None
            )
            .is_ok()
        );
        fs::write(base, dependency_header(None)).unwrap();
        assert!(validate_disk_method("n1", None, &image, images_dir, None).is_ok());
    }

    fn dependency_header(backing: Option<&Path>) -> Vec<u8> {
        let mut bytes = vec![0; 65536];
        bytes[..4].copy_from_slice(b"QFI\xfb");
        bytes[4..8].copy_from_slice(&3_u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&16_u32.to_be_bytes());
        bytes[24..32].copy_from_slice(&(2_u64 << 30).to_be_bytes());
        bytes[100..104].copy_from_slice(&104_u32.to_be_bytes());
        if let Some(backing) = backing {
            let name = backing.to_str().unwrap().as_bytes();
            bytes[8..16].copy_from_slice(&128_u64.to_be_bytes());
            bytes[16..20].copy_from_slice(&u32::try_from(name.len()).unwrap().to_be_bytes());
            bytes[128..128 + name.len()].copy_from_slice(name);
        }
        bytes
    }

    /// A storage pool of these volumes, which a test may add to, for tests.
    #[derive(Default)]
    struct TestPool(Mutex<Vec<BackedVolume>>);

    impl BackedVolumes for TestPool {
        fn backed_volumes(&self) -> Result<Vec<BackedVolume>> {
            Ok(self.0.lock().unwrap().clone())
        }
    }

    fn backed(name: &str, backing_file: &Path) -> BackedVolume {
        BackedVolume {
            name: name.to_owned(),
            backing_file: backing_file.to_path_buf(),
        }
    }

    /// Two versions of a model, each with its base disk.
    fn two_versions(root: &Path) -> (PathBuf, PathBuf) {
        let version = root.join("ubuntu_linux").join("24.04");
        let other = root.join("ubuntu_linux").join("22.04");
        for dir in [&version, &other] {
            fs::create_dir_all(dir).unwrap();
            fs::write(dir.join(VM_DISK_FILENAME), "base").unwrap();
        }
        (version, other)
    }

    #[test]
    fn image_delete_006_an_image_with_a_lab_disk_built_on_it_is_not_deleted_and_the_refusal_names_the_disk()
     {
        let root = tempdir().unwrap();
        let (version, other) = two_versions(root.path());
        let pool = TestPool(Mutex::new(vec![
            backed("vm2-lab-hdd.qcow2", &other.join(VM_DISK_FILENAME)),
            backed("vm1-lab-hdd.qcow2", &version.join(VM_DISK_FILENAME)),
        ]));
        let error = ensure_image_has_no_overlays(&pool, &version).unwrap_err();
        assert!(
            matches!(ServiceError::find(&error), Some(ServiceError::Conflict(_))),
            "{error:#}"
        );
        let message = error.to_string();
        assert!(
            message.contains("Disk vm1-lab-hdd.qcow2 is built on the image"),
            "{message}"
        );
    }

    #[test]
    fn image_delete_007_a_lab_disk_built_on_another_image_does_not_stop_the_delete() {
        let root = tempdir().unwrap();
        let (version, other) = two_versions(root.path());
        let pool = TestPool(Mutex::new(vec![backed(
            "vm2-lab-hdd.qcow2",
            &other.join(VM_DISK_FILENAME),
        )]));
        assert!(ensure_image_has_no_overlays(&pool, &version).is_ok());
        assert!(ensure_image_has_no_overlays(&TestPool::default(), &version).is_ok());
    }

    #[test]
    fn image_delete_009_a_lab_disk_whose_base_is_reached_through_a_renamed_models_folder_stops_the_delete()
     {
        let root = tempdir().unwrap();
        let new = root.path().join("arista_eos");
        let version = new.join("4.33");
        fs::create_dir_all(&version).unwrap();
        fs::write(version.join(VM_DISK_FILENAME), "base").unwrap();
        symlink(&new, root.path().join("arista_veos")).unwrap();
        let old_path = root.path().join("arista_veos/4.33").join(VM_DISK_FILENAME);
        let pool = TestPool(Mutex::new(vec![backed("veos1-lab-hdd.qcow2", &old_path)]));
        assert!(ensure_image_has_no_overlays(&pool, &version).is_err());
    }

    #[test]
    fn image_delete_010_lab_disks_that_cannot_be_read_or_a_base_that_cannot_be_found_refuse_the_delete()
     {
        struct Unreadable;
        impl BackedVolumes for Unreadable {
            fn backed_volumes(&self) -> Result<Vec<BackedVolume>> {
                anyhow::bail!("libvirt is down")
            }
        }
        let root = tempdir().unwrap();
        let (version, _) = two_versions(root.path());
        assert!(ensure_image_has_no_overlays(&Unreadable, &version).is_err());
        let orphan = TestPool(Mutex::new(vec![backed(
            "orphan-hdd.qcow2",
            &root.path().join("gone").join(VM_DISK_FILENAME),
        )]));
        assert!(ensure_image_has_no_overlays(&orphan, &version).is_err());
    }

    #[tokio::test]
    async fn image_delete_016_a_delete_waits_for_an_up_building_a_disk_on_the_image_and_is_refused_by_it()
     {
        // Every lease an up holds keeps the delete waiting, until the last is dropped.
        let first = lease_image(
            ModelKind::default_for(NodeModel::UbuntuLinux),
            "read-leases-test",
        )
        .await;
        let second = lease_image(
            ModelKind::default_for(NodeModel::UbuntuLinux),
            "read-leases-test",
        )
        .await;
        let blocking_task_lease = first.clone();
        let deletion = tokio::spawn(lock_image(
            ModelKind::default_for(NodeModel::UbuntuLinux),
            "read-leases-test",
        ));
        drop(first);
        drop(second);
        tokio::task::yield_now().await;
        assert!(!deletion.is_finished());
        drop(blocking_task_lease);
        let deleted = deletion.await.unwrap();
        drop(deleted);

        // The delete then sees the disk the up built on the image, and is refused.
        let root = tempdir().unwrap();
        let (version, _) = two_versions(root.path());
        let base = version.join(VM_DISK_FILENAME);
        let pool = Arc::new(TestPool::default());
        let provisioning = lock_image(
            ModelKind::default_for(NodeModel::UbuntuLinux),
            "concurrent-overlay-test",
        )
        .await;
        let scan_pool = Arc::clone(&pool);
        let scan_version = version.clone();
        let deletion = tokio::spawn(async move {
            let _guard = lock_image(
                ModelKind::default_for(NodeModel::UbuntuLinux),
                "concurrent-overlay-test",
            )
            .await;
            ensure_image_has_no_overlays(scan_pool.as_ref(), &scan_version)
        });
        tokio::task::yield_now().await;
        assert!(!deletion.is_finished());
        pool.0
            .lock()
            .unwrap()
            .push(backed("new-overlay.qcow2", &base));
        drop(provisioning);
        assert!(deletion.await.unwrap().is_err());
    }
}
