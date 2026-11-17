use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
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

use crate::names::is_lab_volume;

/// A storage volume's XML, as far as its backing file.
#[derive(Debug, Deserialize)]
struct VolumeXml {
    #[serde(rename = "backingStore")]
    backing_store: Option<BackingStoreXml>,
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
    let pool = StoragePool::lookup_by_name(qemu_conn, SHERPA_STORAGE_POOL).context(format!(
        "Failed to find storage pool '{SHERPA_STORAGE_POOL}'"
    ))?;
    refresh_pool(&pool)?;
    let volumes = pool.list_volumes().context(format!(
        "Failed to list volumes in pool '{SHERPA_STORAGE_POOL}'"
    ))?;
    Ok(volumes
        .into_iter()
        .filter(|volume| is_lab_volume(volume, lab_id))
        .collect())
}

/// The backing file of each volume in the Sherpa storage pool that has one, as libvirt reads
/// it from the volume's header after refreshing the pool. libvirt reads every volume, while
/// sherpad cannot read the disks libvirt has handed to QEMU.
///
/// # Errors
///
/// Fails when the pool cannot be found, refreshed or listed, or a volume's XML cannot be read
/// or parsed: a dependency that cannot be determined is an error, not a volume without one.
#[instrument(level = "debug", skip(qemu_conn))]
pub fn pool_backing_files(qemu_conn: &Connect) -> Result<Vec<PathBuf>> {
    let pool = StoragePool::lookup_by_name(qemu_conn, SHERPA_STORAGE_POOL).context(format!(
        "Failed to find storage pool '{SHERPA_STORAGE_POOL}'"
    ))?;
    refresh_pool(&pool)?;
    let volumes = pool.list_all_volumes(0).context(format!(
        "Failed to list volumes in pool '{SHERPA_STORAGE_POOL}'"
    ))?;
    let mut backing_files = Vec::new();
    for volume in volumes {
        let xml = volume
            .get_xml_desc(0)
            .context("Failed to read a storage volume's XML")?;
        if let Some(path) = volume_backing_file(&xml)? {
            backing_files.push(path);
        }
    }
    Ok(backing_files)
}

/// The backing file a storage volume's `xml` records, if any.
fn volume_backing_file(xml: &str) -> Result<Option<PathBuf>> {
    let volume: VolumeXml = from_str(xml).context("Failed to parse a storage volume's XML")?;
    Ok(volume
        .backing_store
        .and_then(|backing| backing.path)
        .map(PathBuf::from))
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
    use std::env;

    use super::*;

    #[test]
    fn storage_pool_xml_escapes_its_name_and_path() {
        let xml = SherpaStoragePool {
            name: "pool&one".to_string(),
            path: "/var/a<b".to_string(),
        }
        .render()
        .unwrap();
        assert!(xml.contains("<name>pool&#38;one</name>"), "{xml}");
        assert!(xml.contains("<path>/var/a&#60;b</path>"), "{xml}");
    }

    #[test]
    fn existing_destinations_are_refused() {
        let directory = env::temp_dir();
        assert!(ensure_destination_absent(directory.to_str().unwrap()).is_err());
        let missing = directory.join("sherpa-libvirt-test-missing-destination");
        assert!(ensure_destination_absent(missing.to_str().unwrap()).is_ok());
    }

    #[test]
    fn a_volumes_backing_file_is_read_from_its_xml() {
        let overlay = "<volume type='file'><name>srx1-lab-hdd.qcow2</name>\
            <target><path>/opt/sherpa/libvirt/images/srx1-lab-hdd.qcow2</path>\
            <format type='qcow2'/></target>\
            <backingStore><path>/opt/sherpa/images/juniper_vsrxv3/23.2/virtioa.qcow2</path>\
            <format type='qcow2'/><permissions><mode>0644</mode></permissions></backingStore>\
            </volume>";
        assert_eq!(
            volume_backing_file(overlay).unwrap(),
            Some(PathBuf::from(
                "/opt/sherpa/images/juniper_vsrxv3/23.2/virtioa.qcow2"
            ))
        );
        let copy = "<volume type='file'><name>srx1-lab.iso</name>\
            <target><path>/opt/sherpa/libvirt/images/srx1-lab.iso</path></target></volume>";
        assert_eq!(volume_backing_file(copy).unwrap(), None);
        assert!(volume_backing_file("<volume").is_err());
    }
}
