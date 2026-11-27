use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use askama::Template;
use quick_xml::de::from_str;
use serde::Deserialize;
use shared::konst::{
    SHERPA_STORAGE_POOL, STORAGE_POOL_REFRESH_ATTEMPTS, STORAGE_POOL_REFRESH_RETRY_MS,
};
use tracing::instrument;
use virt::connect::Connect;
use virt::error::ErrorNumber;
use virt::storage_pool::StoragePool;
use virt::storage_vol::StorageVol;

use crate::names::is_lab_volume;
use crate::size::gib_to_bytes;

#[derive(Template)]
#[template(path = "blank.xml", ext = "xml", escape = "xml")]
struct BlankVolume<'a> {
    name: &'a str,
    destination: &'a str,
    capacity: u64,
}

/// A storage volume's XML, as far as its name, path and backing file.
#[derive(Debug, Deserialize)]
struct VolumeXml {
    name: String,
    target: VolumeTargetXml,
    #[serde(rename = "backingStore")]
    backing_store: Option<BackingStoreXml>,
}

/// A volume's `<target>`: the path of its file.
#[derive(Debug, Deserialize)]
struct VolumeTargetXml {
    path: String,
}

/// A volume in the Sherpa storage pool built on a backing file: an overlay disk and its base.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackedVolume {
    pub name: String,
    pub path: PathBuf,
    pub backing_file: PathBuf,
}

/// A volume's `<backingStore>`: the path of its base, as the volume records it.
#[derive(Debug, Deserialize)]
struct BackingStoreXml {
    path: Option<String>,
}

/// A directory storage pool, defined from `templates/storage_pool.xml`.
#[derive(Debug, Template)]
#[template(path = "storage_pool.xml", ext = "xml", escape = "xml")]
pub struct SherpaStoragePool {
    pub name: String,
    pub path: String,
}

impl SherpaStoragePool {
    /// Make sure the pool is defined, built, running and set to start with libvirt. A pool left
    /// defined but stopped, such as by an earlier start that failed, is built and started.
    ///
    /// # Errors
    ///
    /// Fails when the XML cannot be rendered, or libvirt cannot look up, define, build, start or
    /// autostart the pool.
    #[instrument(level = "debug", skip(self, qemu_conn), fields(pool_name = %self.name))]
    pub fn create(self, qemu_conn: &Connect) -> Result<()> {
        let pool = match StoragePool::lookup_by_name(qemu_conn, &self.name) {
            Ok(pool) => {
                let active = pool.is_active().with_context(|| {
                    format!("Failed to check if storage pool '{}' is active", self.name)
                })?;
                if active {
                    tracing::debug!(pool_name = %self.name, "Storage pool already exists");
                    return Ok(());
                }
                tracing::info!(pool_name = %self.name, "Storage pool exists but is inactive, starting it");
                pool
            }
            Err(error) if error.code() == ErrorNumber::NoStoragePool => {
                let xml = self.render().context("Failed to render storage pool XML")?;
                StoragePool::define_xml(qemu_conn, &xml, 0)
                    .with_context(|| format!("Failed to define storage pool '{}'", self.name))?
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("Failed to look up storage pool '{}'", self.name));
            }
        };

        // Building a directory pool creates its directory and keeps one that exists.
        pool.build(0)
            .with_context(|| format!("Failed to build storage pool '{}'", self.name))?;
        pool.create(0)
            .with_context(|| format!("Failed to start storage pool '{}'", self.name))?;
        pool.set_autostart(true)
            .with_context(|| format!("Failed to set storage pool '{}' autostart", self.name))?;

        tracing::info!(pool_name = %self.name, path = %self.path, "Storage pool created and started");
        Ok(())
    }
}

/// Refresh a storage pool, retrying while another operation holds it.
///
/// libvirt refuses a refresh while asynchronous jobs, such as a volume upload for
/// another node, run on the pool. It reports this only as a generic internal error, so
/// every refresh error is retried for a bounded time and the last one is returned.
///
/// # Errors
///
/// Fails when every attempt fails, with the last attempt's error.
#[instrument(level = "debug", skip(pool))]
pub fn refresh_pool(pool: &StoragePool) -> Result<()> {
    let mut attempt = 1;
    loop {
        match pool.refresh(0) {
            Ok(_) => return Ok(()),
            Err(error) if attempt < STORAGE_POOL_REFRESH_ATTEMPTS => {
                tracing::debug!(attempt, error = %error, "Storage pool busy; retrying refresh");
                thread::sleep(Duration::from_millis(STORAGE_POOL_REFRESH_RETRY_MS));
                attempt += 1;
            }
            Err(error) => {
                return Err(error).context(format!(
                    "Storage pool refresh still failing after {STORAGE_POOL_REFRESH_ATTEMPTS} attempts"
                ));
            }
        }
    }
}

/// The volumes of the Sherpa storage pool that belong to a lab, after refreshing the pool so
/// volumes made outside libvirt are seen. See `is_lab_volume` for how they are matched.
///
/// # Errors
///
/// Fails when the pool cannot be found, refreshed or listed.
#[instrument(level = "debug", skip(qemu_conn))]
pub fn list_lab_volumes(qemu_conn: &Connect, lab_id: &str) -> Result<Vec<String>> {
    Ok(list_pool_volumes(qemu_conn)?
        .into_iter()
        .filter(|volume| is_lab_volume(volume, lab_id))
        .collect())
}

/// The volumes of the Sherpa storage pool, after refreshing the pool so volumes made outside
/// libvirt are seen.
///
/// # Errors
///
/// Fails when the pool cannot be found, refreshed or listed.
#[instrument(level = "debug", skip(qemu_conn))]
pub fn list_pool_volumes(qemu_conn: &Connect) -> Result<Vec<String>> {
    let pool = StoragePool::lookup_by_name(qemu_conn, SHERPA_STORAGE_POOL).context(format!(
        "Failed to find storage pool '{SHERPA_STORAGE_POOL}'"
    ))?;
    refresh_pool(&pool)?;
    pool.list_volumes().context(format!(
        "Failed to list volumes in pool '{SHERPA_STORAGE_POOL}'"
    ))
}

/// Each volume in the Sherpa storage pool that has a backing file, with the file as libvirt
/// reads it from the volume's header after refreshing the pool. libvirt reads every volume,
/// while sherpad cannot read the disks libvirt has handed to QEMU, which are `0600` and owned by
/// `libvirt-qemu` or `root`, running or stopped.
///
/// # Errors
///
/// Fails when the pool cannot be found, refreshed or listed, or a volume's XML cannot be read
/// or parsed: a dependency that cannot be determined is an error, not a volume without one.
#[instrument(level = "debug", skip(qemu_conn))]
pub fn pool_backed_volumes(qemu_conn: &Connect) -> Result<Vec<BackedVolume>> {
    let pool = StoragePool::lookup_by_name(qemu_conn, SHERPA_STORAGE_POOL).context(format!(
        "Failed to find storage pool '{SHERPA_STORAGE_POOL}'"
    ))?;
    refresh_pool(&pool)?;
    let volumes = pool.list_all_volumes(0).context(format!(
        "Failed to list volumes in pool '{SHERPA_STORAGE_POOL}'"
    ))?;
    let mut backed = Vec::new();
    for volume in volumes {
        let xml = volume
            .get_xml_desc(0)
            .context("Failed to read a storage volume's XML")?;
        if let Some(volume) = backed_volume(&xml)? {
            backed.push(volume);
        }
    }
    Ok(backed)
}

/// The volume a storage volume's `xml` describes, if it has a backing file.
fn backed_volume(xml: &str) -> Result<Option<BackedVolume>> {
    let volume: VolumeXml = from_str(xml).context("Failed to parse a storage volume's XML")?;
    Ok(volume
        .backing_store
        .and_then(|backing| backing.path)
        .map(|path| BackedVolume {
            name: volume.name,
            path: PathBuf::from(volume.target.path),
            backing_file: PathBuf::from(path),
        }))
}

/// The first of `volumes`, a storage pool's backed volumes from [`pool_backed_volumes`], built
/// on a file in `image_dir`, running or stopped; `None` when none is. Each backing file is
/// resolved, links included, so a base reached through a renamed model's link at its old path
/// is found too.
///
/// The caller must hold the image lifecycle lock through this check and the deletion, so that
/// no new overlay is built on the image in between.
///
/// # Errors
///
/// Fails when `image_dir` or a backing file cannot be resolved: a dependency that cannot be
/// determined is an error, not a volume without one.
#[instrument(level = "debug", skip(volumes))]
pub fn overlay_on_image<'a>(
    volumes: &'a [BackedVolume],
    image_dir: &Path,
) -> Result<Option<&'a BackedVolume>> {
    let image_dir = fs::canonicalize(image_dir)
        .context("Cannot resolve image directory for dependency check")?;
    for volume in volumes {
        let backing = fs::canonicalize(&volume.backing_file).with_context(|| {
            format!(
                "Cannot resolve the backing file {} of disk {}",
                volume.backing_file.display(),
                volume.name
            )
        })?;
        if backing.starts_with(&image_dir) {
            return Ok(Some(volume));
        }
    }
    Ok(None)
}

/// Create an empty sparse QCOW2 disk of `size_gb` GiB at `dst`, never changing an existing
/// destination.
///
/// # Errors
///
/// Fails when `dst` exists, is relative or is not a UTF-8 file name, or libvirt cannot create
/// the volume.
#[instrument(level = "debug", skip(conn))]
pub fn create_blank_qcow2(conn: &Connect, dst: &str, size_gb: u16) -> Result<()> {
    let xml = BlankVolume {
        name: new_volume_name(dst)?,
        destination: dst,
        capacity: gib_to_bytes(size_gb),
    }
    .render()
    .context("Cannot render blank volume XML")?;
    create_volume(conn, &xml).with_context(|| format!("Cannot create blank QCOW2 disk {dst}"))
}

/// The file name of a new volume at absolute path `dst`, where nothing exists yet.
///
/// # Errors
///
/// Fails when `dst` exists, is relative or has no UTF-8 file name.
pub(crate) fn new_volume_name(dst: &str) -> Result<&str> {
    ensure_destination_absent(dst)?;
    let destination = Path::new(dst);
    ensure!(
        destination.is_absolute(),
        "Disk destination must be absolute: {dst}"
    );
    destination
        .file_name()
        .and_then(|name| name.to_str())
        .with_context(|| format!("Invalid disk file name: {dst}"))
}

/// Create the volume of volume XML `xml` in the Sherpa storage pool, refreshed first, never
/// deleting a volume libvirt refuses as a duplicate, which may be another attempt's.
///
/// # Errors
///
/// Fails when the pool cannot be found or refreshed, or libvirt cannot create the volume.
pub(crate) fn create_volume(conn: &Connect, xml: &str) -> Result<()> {
    let pool = StoragePool::lookup_by_name(conn, SHERPA_STORAGE_POOL)
        .with_context(|| format!("Cannot find storage pool '{SHERPA_STORAGE_POOL}'"))?;
    refresh_pool(&pool)?;
    StorageVol::create_xml(&pool, xml, 0)?;
    Ok(())
}

/// Fail unless nothing exists at `path`. Disks are created exclusively and never replace a file.
pub(crate) fn ensure_destination_absent(path: &str) -> Result<()> {
    ensure!(
        fs::symlink_metadata(path).is_err_and(|error| error.kind() == ErrorKind::NotFound),
        "Disk destination already exists or cannot be inspected: {path}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_delete_008_a_disks_backing_file_is_read_from_its_volume_xml() {
        let overlay = "<volume type='file'><name>srx1-lab-hdd.qcow2</name>\
            <target><path>/opt/sherpa/libvirt/images/srx1-lab-hdd.qcow2</path>\
            <format type='qcow2'/></target>\
            <backingStore><path>/opt/sherpa/images/juniper_vsrxv3/23.2/virtioa.qcow2</path>\
            <format type='qcow2'/><permissions><mode>0644</mode></permissions></backingStore>\
            </volume>";
        assert_eq!(
            backed_volume(overlay).unwrap(),
            Some(BackedVolume {
                name: "srx1-lab-hdd.qcow2".to_owned(),
                path: PathBuf::from("/opt/sherpa/libvirt/images/srx1-lab-hdd.qcow2"),
                backing_file: PathBuf::from("/opt/sherpa/images/juniper_vsrxv3/23.2/virtioa.qcow2"),
            })
        );
        let copy = "<volume type='file'><name>srx1-lab.iso</name>\
            <target><path>/opt/sherpa/libvirt/images/srx1-lab.iso</path></target></volume>";
        assert_eq!(backed_volume(copy).unwrap(), None);
        assert!(backed_volume("<volume").is_err());
    }
}
