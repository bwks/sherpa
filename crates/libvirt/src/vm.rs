use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, ensure};
use askama::Template;
use tracing::instrument;

use virt::connect::Connect;
use virt::domain::Domain;
use virt::storage_pool::StoragePool;
use virt::storage_vol::StorageVol;
use virt::stream::Stream;

use shared::konst::SHERPA_STORAGE_POOL;

#[derive(Template)]
#[template(path = "copy.xml", ext = "xml", escape = "xml")]
struct CopyVolume<'a> {
    name: &'a str,
    destination: &'a str,
    format: &'a str,
}

/// Clone a disk image.
#[instrument(level = "debug", skip(conn))]
pub fn clone_disk(conn: &Connect, src_path: &str, dst_path: &str) -> Result<()> {
    let start = Instant::now();
    ensure!(
        fs::symlink_metadata(dst_path)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
        "Disk destination already exists or cannot be inspected: {dst_path}"
    );
    let mut src_file = File::open(src_path).context("Cannot open disk source")?;
    let file_size = src_file.metadata()?.len();
    let pool = StoragePool::lookup_by_name(conn, SHERPA_STORAGE_POOL)?;
    pool.refresh(0)
        .context("Cannot refresh storage pool before copy")?;

    let file_path = Path::new(dst_path);
    let file_name = file_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Invalid destination path"))?
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid UTF-8 in file name"))?;

    let file_extension = file_path
        .extension()
        .and_then(|ext| ext.to_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid file extension"))?;

    let format_type = match file_extension.to_lowercase().as_str() {
        "raw" => "raw",
        "iso" => "raw",
        "json" => "raw",
        "ign" => "raw",
        "img" => "raw",
        "elf" => "raw",
        "qcow2" => "qcow2",
        _ => {
            return Err(anyhow::anyhow!(
                "Unsupported file extension: {}",
                file_extension
            ));
        }
    };

    let vol_xml = CopyVolume {
        name: file_name,
        destination: dst_path,
        format: format_type,
    }
    .render()?;

    // Create the new volume using the Connect struct
    let new_vol = StorageVol::create_xml(&pool, &vol_xml, 0)?;

    let result = (|| -> Result<()> {
        let stream = Stream::new(conn, 0)?;
        let upload = (|| -> Result<()> {
            new_vol.upload(&stream, 0, file_size, 0)?;
            const CHUNK_SIZE: usize = 25 * 1024 * 1024;
            let mut buffer = vec![0; CHUNK_SIZE];
            loop {
                let bytes_read = src_file.read(&mut buffer)?;
                if bytes_read == 0 {
                    break;
                }
                let mut sent = 0;
                while sent < bytes_read {
                    let count = stream.send(&buffer[sent..bytes_read])?;
                    ensure!(count > 0, "Disk upload stream stopped accepting bytes");
                    sent += count;
                }
            }
            Ok(())
        })();
        match upload {
            Ok(()) => stream.finish().context("Cannot finish disk upload"),
            Err(error) => {
                let _ = stream.abort();
                Err(error)
            }
        }
    })();
    if let Err(error) = result {
        new_vol.delete(0).with_context(|| {
            format!(
                "Disk upload failed ({error:#}); failed to remove newly created volume {dst_path}"
            )
        })?;
        return Err(error).context("Disk copy failed; newly created volume removed");
    }
    tracing::info!(
        disk_creation_seconds = start.elapsed().as_secs_f64(),
        method = "copy",
        "Disk provisioned"
    );

    Ok(())
}

/// Resize a volume in the storage pool via the libvirt API.
#[instrument(level = "debug", skip(conn))]
pub fn resize_disk(conn: &Connect, path: &str, size_gb: u16) -> Result<()> {
    anyhow::ensure!(size_gb > 0, "Disk size must be greater than 0 GB");

    let capacity_bytes = u64::from(size_gb) * 1024 * 1024 * 1024;
    let pool = StoragePool::lookup_by_name(conn, SHERPA_STORAGE_POOL)?;
    let vol = StorageVol::lookup_by_path(conn, path)
        .with_context(|| format!("Volume not found at path: {path}"))?;

    let info = vol
        .get_info()
        .with_context(|| format!("Failed to get volume info for: {path}"))?;

    if capacity_bytes <= info.capacity {
        anyhow::bail!(
            "Requested size {}G ({} bytes) must be larger than current size ({} bytes) for: {}",
            size_gb,
            capacity_bytes,
            info.capacity,
            path
        );
    }

    pool.refresh(0)
        .with_context(|| "Failed to refresh storage pool before resize")?;

    vol.resize(capacity_bytes, 0)
        .with_context(|| format!("Failed to resize volume '{path}' to {size_gb}G"))?;

    Ok(())
}

/// Delete a volume from the storage pool.
#[instrument(level = "debug", skip(conn))]
pub fn delete_disk(conn: &Connect, disk_name: &str) -> Result<()> {
    let pool = StoragePool::lookup_by_name(conn, SHERPA_STORAGE_POOL)?;
    let vol = StorageVol::lookup_by_name(&pool, disk_name)?;
    vol.delete(0)?;
    Ok(())
}

/// Create a virtual machine. This will define a persistent virtual machine.
#[instrument(level = "debug", skip(conn, xml))]
pub fn create_vm(conn: &Connect, xml: &str) -> Result<Domain> {
    let domain = Domain::define_xml(conn, xml)?;
    domain.create()?;
    Ok(domain)
}

/// Get virtual machine's Management IP address
#[instrument(level = "debug", skip(conn))]
pub fn get_mgmt_ip(conn: &Connect, vm_name: &str) -> Result<Option<String>> {
    // Look up the domain by name
    let domain = Domain::lookup_by_name(conn, vm_name)?;

    // Get the network interfaces for the domain
    let interfaces = domain.interface_addresses(0, 0)?;

    // It is assumed that the first IP of the first interface of the VM is the
    // management IP.
    match interfaces.first() {
        Some(interface) => match interface.addrs.first() {
            Some(ip) => Ok(Some(ip.addr.to_string())),
            None => Ok(None),
        },
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_resize_disk_size_gb_to_bytes_conversion() {
        let size_gb: u16 = 100;
        let capacity_bytes = u64::from(size_gb) * 1024 * 1024 * 1024;
        assert_eq!(capacity_bytes, 107_374_182_400);
    }
}
