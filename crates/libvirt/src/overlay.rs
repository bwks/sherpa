use std::fs;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, ensure};
use askama::Template;
use tracing::instrument;
use virt::connect::Connect;
use virt::storage_pool::StoragePool;
use virt::storage_vol::StorageVol;

use crate::qcow2::read_qcow2_metadata;
use shared::konst::SHERPA_STORAGE_POOL;

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
#[instrument(level = "debug", skip(conn))]
pub fn create_overlay(conn: &Connect, src: &str, dst: &str, size_gb: Option<u16>) -> Result<()> {
    let start = Instant::now();
    let backing = fs::canonicalize(src).context("Cannot resolve overlay base")?;
    let capacity = read_qcow2_metadata(&backing)?.overlay_capacity(size_gb)?;
    ensure!(
        fs::symlink_metadata(dst).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
        "Disk destination already exists or cannot be inspected: {dst}"
    );
    let destination = Path::new(dst);
    ensure!(
        destination.is_absolute(),
        "Overlay destination must be absolute"
    );
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .context("Invalid overlay filename")?;
    let backing = backing.to_str().context("Overlay base path is not UTF-8")?;
    let xml = OverlayVolume {
        name,
        destination: dst,
        backing,
        capacity,
    }
    .render()
    .context("Cannot render overlay volume XML")?;
    let pool = StoragePool::lookup_by_name(conn, SHERPA_STORAGE_POOL)
        .context("Cannot find overlay storage pool")?;
    pool.refresh(0)
        .context("Cannot refresh overlay storage pool")?;
    // Libvirt rejects a duplicate volume name atomically. Never delete a volume
    // after create_xml fails: it may belong to a concurrent or previous attempt.
    StorageVol::create_xml(&pool, &xml, 0)
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
    use super::*;
    use crate::{Qemu, QemuConnection, clone_disk, delete_disk, resize_disk};
    use shared::konst::SHERPA_STORAGE_POOL_PATH;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;
    use virt::domain::Domain;
    use virt::stream::Stream;
    use virt::sys::VIR_DOMAIN_UNDEFINE_NVRAM;

    #[test]
    fn overlay_xml_escapes_paths_and_explicitly_sets_backing_format() {
        let xml = OverlayVolume {
            name: "node&one.qcow2",
            destination: "/pool/a<b.qcow2",
            backing: "/images/a&b/base.qcow2",
            capacity: 1234,
        }
        .render()
        .unwrap();
        assert!(xml.contains("node&#38;one.qcow2"));
        assert!(xml.contains("/pool/a&#60;b.qcow2"));
        assert!(xml.contains("/images/a&#38;b/base.qcow2"));
        assert!(xml.contains("<capacity unit='bytes'>1234</capacity>"));
        assert!(xml.contains("<allocation unit='bytes'>0</allocation>"));
        assert_eq!(xml.matches("<format type='qcow2'/>").count(), 2);
    }
    /// Uses only disposable volumes owned by this test. qemu-io is used here to
    /// verify guest-visible blocks; production provisioning never executes QEMU.
    #[test]
    #[ignore = "Requires local libvirt, sherpa-pool and qemu-io; set SHERPA_OVERLAY_LIVE=1"]
    fn live_overlay_disk_isolation_resize_and_reset() -> Result<()> {
        ensure!(
            std::env::var("SHERPA_OVERLAY_LIVE").as_deref() == Ok("1"),
            "Set SHERPA_OVERLAY_LIVE=1 to authorize disposable test volumes"
        );
        let mut fixture = LiveVolumes::new()?;
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
        let first = fixture.path("first.qcow2");
        let second = fixture.path("second.qcow2");
        fixture.overlay(&base, "first.qcow2", None)?;
        fixture.overlay(&base, "second.qcow2", Some(1))?;
        block_io(&fixture.conn, &first, "read -P 0x5a 0 4096")?;
        block_io(&fixture.conn, &second, "read -P 0x5a 0 4096")?;
        block_io(&fixture.conn, &first, "write -P 0xa5 0 4096")?;
        block_io(&fixture.conn, &second, "read -P 0x5a 0 4096")?;
        block_io(&fixture.conn, &second, "write -P 0x33 0 4096")?;
        block_io(&fixture.conn, &first, "read -P 0xa5 0 4096")?;
        assert_eq!(fs::read(&base)?, original);

        // A stop/start opens the same disk; writes must survive resume.
        let domain_xml = TestDomain {
            name: &fixture.prefix,
            disk: &first,
            memory: 256,
            efi: false,
        }
        .render()?;
        let domain = Domain::define_xml(&fixture.conn, &domain_xml)?;
        fixture.domain = Some(domain);
        let domain = fixture.domain.as_ref().context("Missing test domain")?;
        domain.create()?;
        domain.destroy()?;
        block_io(&fixture.conn, &first, "read -P 0xa5 0 4096")?;
        domain.create()?;
        domain.destroy()?;
        block_io(&fixture.conn, &first, "read -P 0xa5 0 4096")?;
        domain.undefine_flags(VIR_DOMAIN_UNDEFINE_NVRAM)?;
        fixture.domain = None;

        resize_disk(&fixture.conn, &first, 2)?;
        assert_eq!(read_qcow2_metadata(Path::new(&first))?.capacity, 2 << 30);
        assert_eq!(read_qcow2_metadata(Path::new(&base))?.capacity, 1 << 30);
        assert_eq!(fs::read(&base)?, original);
        // Neither copy nor overlay attempts may replace an existing disk.
        assert!(create_overlay(&fixture.conn, &base, &first, None).is_err());
        assert!(clone_disk(&fixture.conn, &base, &first).is_err());
        block_io(&fixture.conn, &first, "read -P 0xa5 0 4096")?;
        delete_disk(&fixture.conn, &fixture.name("first.qcow2"))?;
        let first_name = fixture.name("first.qcow2");
        fixture.created.retain(|name| name != &first_name);
        fixture.overlay(&base, "first.qcow2", None)?;
        block_io(&fixture.conn, &first, "read -P 0x5a 0 4096")?;
        assert_eq!(fs::read(&base)?, original);

        let grown = fixture.path("grown.qcow2");
        fixture.overlay(&base, "grown.qcow2", Some(2))?;
        assert_eq!(read_qcow2_metadata(Path::new(&grown))?.capacity, 2 << 30);
        let failed_copy = fixture.path("failed-copy.qcow2");
        assert!(clone_disk(&fixture.conn, SHERPA_STORAGE_POOL_PATH, &failed_copy).is_err());
        assert!(
            !Path::new(&failed_copy).exists(),
            "Failed upload must remove its new volume"
        );

        let copy = fixture.path("copy.qcow2");
        clone_disk(&fixture.conn, &base, &copy)?;
        fixture.created.push(fixture.name("copy.qcow2"));
        block_io(&fixture.conn, &copy, "read -P 0x5a 0 4096")?;
        // Configuration media uses its existing byte-copy path.
        let media = std::env::temp_dir()
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

    #[test]
    #[ignore = "Requires libvirt and an immutable Omarchy image; set SHERPA_OVERLAY_BENCH_BASE"]
    fn live_omarchy_provisioning_benchmark() -> Result<()> {
        let base = std::env::var("SHERPA_OVERLAY_BENCH_BASE")
            .context("Set SHERPA_OVERLAY_BENCH_BASE to the immutable Omarchy QCOW2 image")?;
        read_qcow2_metadata(Path::new(&base))?.overlay_capacity(None)?;
        let mut fixture = LiveVolumes::new()?;
        for method in ["copy", "overlay"] {
            let suffix = format!("{method}.qcow2");
            let disk = fixture.path(&suffix);
            let creation = Instant::now();
            if method == "copy" {
                clone_disk(&fixture.conn, &base, &disk)?;
            } else {
                create_overlay(&fixture.conn, &base, &disk, None)?;
            }
            fixture.created.push(fixture.name(&suffix));
            eprintln!(
                "{method}: disk creation took {:.6}s",
                creation.elapsed().as_secs_f64()
            );
            let xml = TestDomain {
                name: &fixture.prefix,
                disk: &disk,
                memory: 8192,
                efi: true,
            }
            .render()?;
            fixture.domain = Some(Domain::define_xml(&fixture.conn, &xml)?);
            let domain = fixture
                .domain
                .as_ref()
                .context("Missing benchmark domain")?;
            let startup = Instant::now();
            domain.create()?;
            eprintln!(
                "{method}: domain startup took {:.6}s",
                startup.elapsed().as_secs_f64()
            );
            let polling = Instant::now();
            let mut ready = false;
            while polling.elapsed() < Duration::from_secs(90) {
                if domain
                    .qemu_agent_command(r#"{"execute":"guest-ping"}"#, 1, 0)
                    .is_ok()
                {
                    ready = true;
                    break;
                }
                std::thread::sleep(Duration::from_secs(1));
            }
            eprintln!(
                "{method}: guest-agent readiness polling took {:.6}s; ready={ready}",
                polling.elapsed().as_secs_f64()
            );
            domain.destroy()?;
            domain.undefine_flags(VIR_DOMAIN_UNDEFINE_NVRAM)?;
            fixture.domain = None;
        }
        Ok(())
    }

    #[derive(Template)]
    #[template(path = "test-domain.xml", ext = "xml", escape = "xml")]
    struct TestDomain<'a> {
        name: &'a str,
        disk: &'a str,
        memory: u32,
        efi: bool,
    }

    struct LiveVolumes {
        conn: QemuConnection,
        prefix: String,
        created: Vec<String>,
        files: Vec<String>,
        domain: Option<Domain>,
    }

    impl LiveVolumes {
        fn new() -> Result<Self> {
            let prefix = format!(
                "sherpa-overlay-test-{}-{}",
                std::process::id(),
                NEXT_TEST.fetch_add(1, Ordering::Relaxed)
            );
            Ok(Self {
                conn: Qemu::default().connect()?,
                prefix,
                created: vec![],
                files: vec![],
                domain: None,
            })
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
            if let Some(domain) = self.domain.take() {
                if domain.is_active().unwrap_or(true) {
                    let _ = domain.destroy();
                }
                if let Err(error) = domain.undefine_flags(VIR_DOMAIN_UNDEFINE_NVRAM) {
                    eprintln!("Test domain cleanup failed: {error}");
                    return;
                }
            }
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

    static NEXT_TEST: AtomicU64 = AtomicU64::new(0);

    // Test guest writes on a user-owned temporary QCOW2 file, then upload the
    // resulting image through libvirt. This needs no sudo or writable pool files.
    fn block_io(conn: &Connect, path: &str, command: &str) -> Result<()> {
        let writing = command.starts_with("write ");
        let temporary = std::env::temp_dir().join(format!(
            "sherpa-overlay-io-{}-{}.qcow2",
            std::process::id(),
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
        let result = (|| -> Result<()> {
            ensure!(
                output.status.success(),
                "Guest block check failed: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            if writing {
                let bytes = fs::read(&temporary)?;
                let volume = StorageVol::lookup_by_path(conn, path)?;
                let stream = Stream::new(conn, 0)?;
                volume.upload(&stream, 0, bytes.len() as u64, 0)?;
                let mut sent = 0;
                while sent < bytes.len() {
                    let count = stream.send(&bytes[sent..])?;
                    ensure!(count > 0, "Test upload stopped accepting bytes");
                    sent += count;
                }
                stream.finish()?;
            }
            Ok(())
        })();
        if writing {
            let _ = fs::remove_file(temporary);
        }
        result
    }
}
