use std::fs;
use std::time::Instant;

use anyhow::{Context, Result};
use askama::Template;
use tracing::instrument;
use virt::connect::Connect;

use crate::qcow2::read_qcow2_metadata;
use crate::storage::{create_volume, new_volume_name};

#[derive(Template)]
#[template(path = "overlay.xml", ext = "xml", escape = "xml")]
struct OverlayVolume<'a> {
    name: &'a str,
    destination: &'a str,
    backing: &'a str,
    capacity: u64,
}

/// Create a sparse QCOW2 boot disk with an absolute, explicit QCOW2 backing store.
/// The volume is created exclusively; existing destinations are never changed.
///
/// # Errors
///
/// Fails when the base cannot be resolved or is not a supported standalone QCOW2 image, the
/// size would shrink it, `dst` exists, is relative or is not a UTF-8 file name, or libvirt
/// cannot create the volume.
#[instrument(level = "debug", skip(conn))]
pub fn create_overlay(conn: &Connect, src: &str, dst: &str, size_gb: Option<u16>) -> Result<()> {
    let start = Instant::now();
    let backing = fs::canonicalize(src).context("Cannot resolve overlay base")?;
    let capacity = read_qcow2_metadata(&backing)?.overlay_capacity(size_gb)?;
    let name = new_volume_name(dst)?;
    let backing = backing.to_str().context("Overlay base path is not UTF-8")?;
    let xml = OverlayVolume {
        name,
        destination: dst,
        backing,
        capacity,
    }
    .render()
    .context("Cannot render overlay volume XML")?;
    create_volume(conn, &xml)
        .with_context(|| format!("Cannot create QCOW2 overlay {dst} backed by {backing}"))?;
    tracing::info!(
        disk_creation_seconds = start.elapsed().as_secs_f64(),
        method = "overlay",
        "Boot disk provisioned"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::{self, Command, Output};
    use std::sync::atomic::{AtomicU64, Ordering};

    use anyhow::ensure;
    use shared::konst::{SHERPA_STORAGE_POOL, SHERPA_STORAGE_POOL_PATH};
    use virt::storage_pool::StoragePool;
    use virt::storage_vol::StorageVol;
    use virt::stream::Stream;

    use super::*;
    use crate::{Qemu, QemuConnection, clone_disk, delete_disk, resize_disk};

    static NEXT_TEST: AtomicU64 = AtomicU64::new(0);

    /// Uses only disposable volumes owned by this test. qemu-io is used here to
    /// verify guest-visible blocks; production provisioning never executes QEMU.
    #[test]
    #[ignore = "Requires local libvirt, sherpa-pool and qemu-io; set SHERPA_OVERLAY_LIVE=1"]
    fn vm_start_001_what_a_vm_writes_to_its_overlay_changes_neither_the_base_nor_another_disk()
    -> Result<()> {
        let (mut fixture, base, original) = LiveVolumes::with_base()?;
        let first = fixture.path("first.qcow2");
        let second = fixture.path("second.qcow2");
        fixture.overlay(&base, "first.qcow2", None)?;
        fixture.overlay(&base, "second.qcow2", None)?;
        block_io(&fixture.conn, &first, "read -P 0x5a 0 4096")?;
        block_io(&fixture.conn, &second, "read -P 0x5a 0 4096")?;
        block_io(&fixture.conn, &first, "write -P 0xa5 0 4096")?;
        block_io(&fixture.conn, &second, "read -P 0x5a 0 4096")?;
        block_io(&fixture.conn, &second, "write -P 0x33 0 4096")?;
        block_io(&fixture.conn, &first, "read -P 0xa5 0 4096")?;
        assert_eq!(fs::read(&base)?, original);
        Ok(())
    }

    #[test]
    #[ignore = "Requires local libvirt, sherpa-pool and qemu-io; set SHERPA_OVERLAY_LIVE=1"]
    fn vm_start_002_a_boot_disk_size_gives_the_disk_that_size_and_leaves_the_base_as_it_was()
    -> Result<()> {
        let (mut fixture, base, original) = LiveVolumes::with_base()?;
        let grown = fixture.path("grown.qcow2");
        fixture.overlay(&base, "grown.qcow2", Some(2))?;
        assert_eq!(read_qcow2_metadata(Path::new(&grown))?.capacity, 2 << 30);
        let resized = fixture.path("resized.qcow2");
        fixture.overlay(&base, "resized.qcow2", None)?;
        resize_disk(&fixture.conn, &resized, 2)?;
        assert_eq!(read_qcow2_metadata(Path::new(&resized))?.capacity, 2 << 30);
        assert_eq!(read_qcow2_metadata(Path::new(&base))?.capacity, 1 << 30);
        assert_eq!(fs::read(&base)?, original);
        Ok(())
    }

    #[test]
    #[ignore = "Requires local libvirt, sherpa-pool and qemu-io; set SHERPA_OVERLAY_LIVE=1"]
    fn vm_start_004_a_copied_boot_disk_has_the_base_images_contents() -> Result<()> {
        let (mut fixture, base, _) = LiveVolumes::with_base()?;
        let copy = fixture.path("copy.qcow2");
        clone_disk(&fixture.conn, &base, &copy)?;
        fixture.created.push(fixture.name("copy.qcow2"));
        block_io(&fixture.conn, &copy, "read -P 0x5a 0 4096")?;
        Ok(())
    }

    #[test]
    #[ignore = "Requires local libvirt, sherpa-pool and qemu-io; set SHERPA_OVERLAY_LIVE=1"]
    fn vm_start_005_a_boot_disk_is_never_created_over_an_existing_disk() -> Result<()> {
        let (mut fixture, base, _) = LiveVolumes::with_base()?;
        let first = fixture.path("first.qcow2");
        fixture.overlay(&base, "first.qcow2", None)?;
        block_io(&fixture.conn, &first, "write -P 0xa5 0 4096")?;
        assert!(create_overlay(&fixture.conn, &base, &first, None).is_err());
        assert!(clone_disk(&fixture.conn, &base, &first).is_err());
        block_io(&fixture.conn, &first, "read -P 0xa5 0 4096")?;
        Ok(())
    }

    #[test]
    #[ignore = "Requires local libvirt, sherpa-pool and qemu-io; set SHERPA_OVERLAY_LIVE=1"]
    fn vm_start_006_a_copy_that_fails_leaves_no_new_volume_behind() -> Result<()> {
        let (fixture, _, _) = LiveVolumes::with_base()?;
        let failed_copy = fixture.path("failed-copy.qcow2");
        assert!(clone_disk(&fixture.conn, SHERPA_STORAGE_POOL_PATH, &failed_copy).is_err());
        assert!(
            !Path::new(&failed_copy).exists(),
            "Failed upload must remove its new volume"
        );
        Ok(())
    }

    /// Configuration media uses its existing byte-copy path. Left for the ZTP spec.
    #[test]
    #[ignore = "Requires local libvirt and sherpa-pool; set SHERPA_OVERLAY_LIVE=1"]
    fn config_delivery_006_a_config_medium_is_copied_into_the_pool_byte_for_byte() -> Result<()> {
        let mut fixture = LiveVolumes::new()?;
        let media = env::temp_dir()
            .join(fixture.name("source.iso"))
            .to_string_lossy()
            .into_owned();
        fs::write(&media, b"auxiliary configuration media")?;
        fixture.files.push(media.clone());
        let copied_media = fixture.path("copy.iso");
        clone_disk(&fixture.conn, &media, &copied_media)?;
        fixture.created.push(fixture.name("copy.iso"));
        assert_eq!(fs::read(copied_media)?, b"auxiliary configuration media");
        Ok(())
    }

    /// Every volume a node gets in the pool is private, whatever the mode of its source: an
    /// overlay boot disk, a copied boot disk and its config media.
    #[test]
    #[ignore = "Requires local libvirt and sherpa-pool; set SHERPA_OVERLAY_LIVE=1"]
    fn vm_start_010_a_nodes_disks_and_config_media_are_private() -> Result<()> {
        let (mut fixture, base, _) = LiveVolumes::with_base()?;
        assert_eq!(
            mode_of(&base)?,
            0o644,
            "the base is not wider than its copies"
        );
        fixture.overlay(&base, "overlay.qcow2", None)?;
        let mut volumes = vec![fixture.path("overlay.qcow2")];
        clone_disk(&fixture.conn, &base, &fixture.path("copy.qcow2"))?;
        fixture.created.push(fixture.name("copy.qcow2"));
        volumes.push(fixture.path("copy.qcow2"));
        for medium in ["config.iso", "config-cfg.img"] {
            let source = env::temp_dir()
                .join(fixture.name(medium))
                .to_string_lossy()
                .into_owned();
            fs::write(&source, b"config")?;
            fs::set_permissions(&source, fs::Permissions::from_mode(0o644))?;
            fixture.files.push(source.clone());
            clone_disk(&fixture.conn, &source, &fixture.path(medium))?;
            fixture.created.push(fixture.name(medium));
            volumes.push(fixture.path(medium));
        }
        for volume in volumes {
            assert_eq!(mode_of(&volume)?, 0o600, "{volume}");
        }
        Ok(())
    }

    /// The permission bits of the file at `path`.
    fn mode_of(path: &str) -> Result<u32> {
        Ok(fs::metadata(path)?.permissions().mode() & 0o7777)
    }

    struct LiveVolumes {
        conn: QemuConnection,
        prefix: String,
        created: Vec<String>,
        files: Vec<String>,
    }

    impl LiveVolumes {
        fn new() -> Result<Self> {
            ensure!(
                env::var("SHERPA_OVERLAY_LIVE").as_deref() == Ok("1"),
                "Set SHERPA_OVERLAY_LIVE=1 to authorize disposable test volumes"
            );
            let prefix = format!(
                "sherpa-overlay-test-{}-{}",
                process::id(),
                NEXT_TEST.fetch_add(1, Ordering::Relaxed)
            );
            Ok(Self {
                conn: Qemu::default().connect()?,
                prefix,
                created: vec![],
                files: vec![],
            })
        }

        /// A 1 GiB base volume whose first block is written with 0x5a, its path and its bytes.
        fn with_base() -> Result<(Self, String, Vec<u8>)> {
            let mut fixture = Self::new()?;
            let pool = StoragePool::lookup_by_name(&fixture.conn, SHERPA_STORAGE_POOL)?;
            let base_name = fixture.name("base.qcow2");
            let base = fixture.path("base.qcow2");
            let xml = format!(
                "<volume><name>{base_name}</name><capacity unit='bytes'>1073741824</capacity><allocation>0</allocation><target><format type='qcow2'/><permissions><mode>0644</mode></permissions></target></volume>"
            );
            StorageVol::create_xml(&pool, &xml, 0)?;
            fixture.created.push(base_name);
            block_io(&fixture.conn, &base, "write -P 0x5a 0 4096")?;
            let original = fs::read(&base)?;
            Ok((fixture, base, original))
        }
        fn name(&self, suffix: &str) -> String {
            format!("{}-{suffix}", self.prefix)
        }
        fn path(&self, suffix: &str) -> String {
            format!("{SHERPA_STORAGE_POOL_PATH}/{}", self.name(suffix))
        }
        fn overlay(&mut self, base: &str, suffix: &str, size: Option<u16>) -> Result<()> {
            create_overlay(&self.conn, base, &self.path(suffix), size)?;
            self.created.push(self.name(suffix));
            Ok(())
        }
    }

    impl Drop for LiveVolumes {
        fn drop(&mut self) {
            for name in self.created.iter().rev() {
                if let Err(error) = delete_disk(&self.conn, name) {
                    eprintln!("Test volume cleanup failed for {name}: {error}");
                    return;
                }
            }
            for path in &self.files {
                let _ = fs::remove_file(path);
            }
        }
    }

    // Test guest writes on a user-owned temporary QCOW2 file, then upload the
    // resulting image through libvirt. This needs no sudo or writable pool files.
    fn block_io(conn: &Connect, path: &str, command: &str) -> Result<()> {
        let writing = command.starts_with("write ");
        let temporary = env::temp_dir().join(format!(
            "sherpa-overlay-io-{}-{}.qcow2",
            process::id(),
            NEXT_TEST.fetch_add(1, Ordering::Relaxed)
        ));
        let local_path = if writing {
            fs::copy(path, &temporary)?;
            temporary.to_str().context("Invalid temporary image path")?
        } else {
            path
        };
        let mut process = Command::new("qemu-io");
        if !writing {
            process.arg("-r");
        }
        let output = process
            .args(["-f", "qcow2", "-c", command, local_path])
            .output()?;
        let result = finish_block_io(conn, path, &temporary, &output, writing);
        if writing {
            let _ = fs::remove_file(temporary);
        }
        result
    }

    /// Fail unless qemu-io succeeded, and upload a written image over the pool volume at `path`.
    fn finish_block_io(
        conn: &Connect,
        path: &str,
        temporary: &Path,
        output: &Output,
        writing: bool,
    ) -> Result<()> {
        ensure!(
            output.status.success(),
            "Guest block check failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !writing {
            return Ok(());
        }
        let bytes = fs::read(temporary)?;
        let volume = StorageVol::lookup_by_path(conn, path)?;
        let stream = Stream::new(conn, 0)?;
        volume.upload(&stream, 0, u64::try_from(bytes.len())?, 0)?;
        let mut sent = 0;
        while sent < bytes.len() {
            let count = stream.send(&bytes[sent..])?;
            ensure!(count > 0, "Test upload stopped accepting bytes");
            sent += count;
        }
        stream.finish()?;
        Ok(())
    }
}
