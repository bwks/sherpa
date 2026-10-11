use std::path::Path;
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result, ensure};
use dashmap::DashMap;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use tracing::instrument;

use shared::data::{DiskImageCloneMethod, NodeConfig, NodeKind, NodeModel};
use shared::util::image_filename;

pub(crate) type ImageLease = Arc<OwnedRwLockReadGuard<()>>;

static IMAGE_LOCKS: OnceLock<DashMap<String, Arc<RwLock<()>>>> = OnceLock::new();

/// Shared across AppState instances and all image-mutating workflows in this process.
#[instrument(level = "debug")]
pub(crate) async fn lock_image(model: NodeModel, version: &str) -> OwnedRwLockWriteGuard<()> {
    let key = format!("{model}/{version}");
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
pub(crate) async fn lease_image(model: NodeModel, version: &str) -> ImageLease {
    let key = format!("{model}/{version}");
    let lock = IMAGE_LOCKS
        .get_or_init(DashMap::new)
        .entry(key)
        .or_insert_with(|| Arc::new(RwLock::new(())))
        .clone();
    Arc::new(lock.read_owned().await)
}

/// Validate policy and base before any resource creation or redeploy teardown.
#[instrument(level = "debug", skip(image))]
pub(crate) fn validate_disk_method(
    method: Option<DiskImageCloneMethod>,
    image: &NodeConfig,
    images_dir: &str,
    size_gb: Option<u16>,
) -> Result<()> {
    if image.kind != NodeKind::VirtualMachine {
        ensure!(
            method != Some(DiskImageCloneMethod::Overlay),
            "Overlay provisioning is supported only for VM boot disks"
        );
        return Ok(());
    }
    if method.unwrap_or_default() == DiskImageCloneMethod::Copy {
        return Ok(());
    }
    let filename = image_filename(&image.kind, image.boot_mode.as_ref());
    let path = Path::new(images_dir)
        .join(image.model.to_string())
        .join(&image.version)
        .join(filename);
    libvirt::read_qcow2_metadata(&path)?
        .overlay_capacity(size_gb)
        .context("VM boot image is ineligible for overlay provisioning")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn lifecycle_lock_serializes_same_image_and_allows_other_versions() {
        let held = lock_image(NodeModel::UbuntuLinux, "lock-test-one").await;
        let waiting = tokio::spawn(lock_image(NodeModel::UbuntuLinux, "lock-test-one"));
        let other = lock_image(NodeModel::UbuntuLinux, "lock-test-two").await;
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        drop(held);
        let acquired = waiting.await.unwrap();
        drop(acquired);
        drop(other);
    }

    #[test]
    fn unsupported_kinds_do_not_fall_back_to_copy() {
        let mut image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        for kind in [NodeKind::Container, NodeKind::Unikernel] {
            image.kind = kind;
            assert!(validate_disk_method(None, &image, "/missing", None).is_ok());
            assert!(
                validate_disk_method(
                    Some(DiskImageCloneMethod::Overlay),
                    &image,
                    "/missing",
                    None
                )
                .is_err()
            );
            assert!(
                validate_disk_method(Some(DiskImageCloneMethod::Copy), &image, "/missing", None)
                    .is_ok()
            );
        }
    }
    #[test]
    fn omitted_vm_method_validates_the_base_as_an_overlay() {
        let root = tempdir().unwrap();
        let image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        let directory = root
            .path()
            .join(image.model.to_string())
            .join(&image.version);
        fs::create_dir_all(&directory).unwrap();
        let base = directory.join(image_filename(&image.kind, image.boot_mode.as_ref()));
        fs::write(&base, "not a QCOW2 image").unwrap();
        let images_dir = root.path().to_str().unwrap();
        assert!(validate_disk_method(None, &image, images_dir, None).is_err());
        assert!(
            validate_disk_method(Some(DiskImageCloneMethod::Copy), &image, images_dir, None)
                .is_ok()
        );
        fs::write(base, dependency_header(None)).unwrap();
        assert!(validate_disk_method(None, &image, images_dir, None).is_ok());
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
            bytes[16..20].copy_from_slice(&(name.len() as u32).to_be_bytes());
            bytes[128..128 + name.len()].copy_from_slice(name);
        }
        bytes
    }

    #[test]
    fn stopped_and_orphaned_overlays_protect_base_after_restart() {
        let root = tempdir().unwrap();
        let storage = root.path().join("pool");
        let version = root.path().join("version");
        fs::create_dir(&storage).unwrap();
        fs::create_dir(&version).unwrap();
        let base = version.join("disk.qcow2");
        fs::write(&base, dependency_header(None)).unwrap();
        fs::write(storage.join("orphan.qcow2"), dependency_header(Some(&base))).unwrap();
        // No database record, running domain or in-memory dependency registry.
        // Repeated fresh scans model restart and must reach the same decision.
        for _ in 0..2 {
            assert!(libvirt::ensure_no_overlay_dependencies(&storage, &version).is_err());
        }
        fs::remove_file(storage.join("orphan.qcow2")).unwrap();
        assert!(libvirt::ensure_no_overlay_dependencies(&storage, &version).is_ok());
        assert!(base.exists());
    }

    #[test]
    fn unreadable_or_missing_dependency_metadata_blocks_deletion() {
        let root = tempdir().unwrap();
        let storage = root.path().join("pool");
        let version = root.path().join("version");
        fs::create_dir(&storage).unwrap();
        fs::create_dir(&version).unwrap();
        fs::write(storage.join("broken.qcow2"), "truncated").unwrap();
        assert!(libvirt::ensure_no_overlay_dependencies(&storage, &version).is_err());
        fs::write(
            storage.join("broken.qcow2"),
            dependency_header(Some(&version.join("missing.qcow2"))),
        )
        .unwrap();
        assert!(libvirt::ensure_no_overlay_dependencies(&storage, &version).is_err());
    }

    #[tokio::test]
    async fn deletion_waits_for_provisioning_then_observes_overlay() {
        let root = tempdir().unwrap();
        let storage = root.path().join("pool");
        let version = root.path().join("version");
        fs::create_dir(&storage).unwrap();
        fs::create_dir(&version).unwrap();
        let base = version.join("disk.qcow2");
        fs::write(&base, dependency_header(None)).unwrap();
        let provisioning = lock_image(NodeModel::UbuntuLinux, "concurrent-overlay-test").await;
        let scan_storage = storage.clone();
        let scan_version = version.clone();
        let deletion = tokio::spawn(async move {
            let _guard = lock_image(NodeModel::UbuntuLinux, "concurrent-overlay-test").await;
            libvirt::ensure_no_overlay_dependencies(&scan_storage, &scan_version)
        });
        tokio::task::yield_now().await;
        assert!(!deletion.is_finished());
        fs::write(
            storage.join("new-overlay.qcow2"),
            dependency_header(Some(&base)),
        )
        .unwrap();
        drop(provisioning);
        assert!(deletion.await.unwrap().is_err());
        assert!(base.exists());
    }
    #[test]
    fn manifest_file_writer_retains_node_disk_methods() {
        let root = tempdir().unwrap();
        let file = root.path().join("manifest.toml");
        let manifest: topology::Manifest = toml::from_str(
            "name='test'\nnodes=[{name='vm',model='ubuntu_linux',boot_disk_clone_method='copy'}]",
        )
        .unwrap();
        manifest.write_file(file.to_str().unwrap()).unwrap();
        let saved = topology::Manifest::load_file(file.to_str().unwrap()).unwrap();
        assert_eq!(
            saved.nodes[0].boot_disk_clone_method,
            Some(DiskImageCloneMethod::Copy)
        );
    }
    #[tokio::test]
    async fn concurrent_provisioning_leases_exclude_image_deletion() {
        let first = lease_image(NodeModel::UbuntuLinux, "read-leases-test").await;
        let second = lease_image(NodeModel::UbuntuLinux, "read-leases-test").await;
        let blocking_task_lease = first.clone();
        let deletion = tokio::spawn(lock_image(NodeModel::UbuntuLinux, "read-leases-test"));
        drop(first);
        drop(second);
        tokio::task::yield_now().await;
        assert!(!deletion.is_finished());
        drop(blocking_task_lease);
        let deleted = deletion.await.unwrap();
        drop(deleted);
    }
}
