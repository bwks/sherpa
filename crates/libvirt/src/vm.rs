use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail, ensure};
use askama::Template;
use tracing::instrument;

use virt::connect::Connect;
use virt::domain::Domain;
use virt::storage_pool::StoragePool;
use virt::storage_vol::StorageVol;
use virt::stream::Stream;

use shared::konst::{SHERPA_STORAGE_POOL, STORAGE_VOLUME_MODE, STORAGE_VOLUME_QEMU_ARG_MODE};

use crate::size::gib_to_bytes;
use crate::storage::{ensure_destination_absent, refresh_pool};

/// Bytes read from a source disk and sent to libvirt at a time.
const UPLOAD_CHUNK_SIZE: usize = 25 * 1024 * 1024;

#[derive(Template)]
#[template(path = "copy.xml", ext = "xml", escape = "xml")]
struct CopyVolume<'a> {
    name: &'a str,
    destination: &'a str,
    format: &'a str,
    mode: &'a str,
}

/// The libvirt volume format and file mode for a file copied into the storage pool.
fn volume_format_and_mode(file_extension: &str) -> Result<(&'static str, &'static str)> {
    let format = match file_extension.to_lowercase().as_str() {
        "raw" | "iso" | "json" | "ign" | "img" | "elf" => "raw",
        "qcow2" => "qcow2",
        _ => bail!("Unsupported file extension: {file_extension}"),
    };
    // QEMU opens Flatcar's Ignition file itself, from a raw `-fw_cfg` argument.
    let mode = if file_extension.eq_ignore_ascii_case("ign") {
        STORAGE_VOLUME_QEMU_ARG_MODE
    } else {
        STORAGE_VOLUME_MODE
    };
    Ok((format, mode))
}

/// Copy the file `src_path` into the storage pool as the new volume `dst_path`. The volume is
/// removed again if the copy fails.
///
/// # Errors
///
/// Fails when `dst_path` already exists or cannot be inspected, the source cannot be read, the
/// destination's extension is not a supported format, or libvirt cannot create the volume or
/// take the upload.
#[instrument(level = "debug", skip(conn))]
pub fn clone_disk(conn: &Connect, src_path: &str, dst_path: &str) -> Result<()> {
    let start = Instant::now();
    ensure_destination_absent(dst_path)?;
    let mut src_file =
        File::open(src_path).with_context(|| format!("Cannot open disk source {src_path}"))?;
    let file_size = src_file
        .metadata()
        .with_context(|| format!("Cannot read the size of disk source {src_path}"))?
        .len();
    let pool = StoragePool::lookup_by_name(conn, SHERPA_STORAGE_POOL)
        .with_context(|| format!("Cannot find storage pool '{SHERPA_STORAGE_POOL}'"))?;
    refresh_pool(&pool).context("Cannot refresh storage pool before copy")?;

    let file_path = Path::new(dst_path);
    let file_name = file_path
        .file_name()
        .context("Invalid destination path")?
        .to_str()
        .context("Invalid UTF-8 in file name")?;
    let file_extension = file_path
        .extension()
        .and_then(|ext| ext.to_str())
        .context("Invalid file extension")?;

    let (format_type, mode) = volume_format_and_mode(file_extension)?;

    let vol_xml = CopyVolume {
        name: file_name,
        destination: dst_path,
        format: format_type,
        mode,
    }
    .render()
    .context("Cannot render disk volume XML")?;

    let new_vol = StorageVol::create_xml(&pool, &vol_xml, 0)
        .with_context(|| format!("Cannot create volume {dst_path}"))?;
    if let Err(error) = upload_volume(conn, &new_vol, &mut src_file, file_size) {
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

/// Upload `size` bytes of `source` into `volume`, aborting the stream if sending fails.
fn upload_volume(conn: &Connect, volume: &StorageVol, source: &mut File, size: u64) -> Result<()> {
    let stream = Stream::new(conn, 0).context("Cannot open a disk upload stream")?;
    match send_file(volume, &stream, source, size) {
        Ok(()) => stream.finish().context("Cannot finish disk upload"),
        Err(error) => {
            if let Err(abort) = stream.abort() {
                tracing::warn!(error = %abort, "Cannot abort disk upload stream");
            }
            Err(error)
        }
    }
}

/// Start an upload of `size` bytes into `volume` on `stream`, and send all of `source`.
fn send_file(volume: &StorageVol, stream: &Stream, source: &mut File, size: u64) -> Result<()> {
    volume
        .upload(stream, 0, size, 0)
        .context("Cannot start disk upload")?;
    let mut buffer = vec![0; UPLOAD_CHUNK_SIZE];
    loop {
        let bytes_read = source
            .read(&mut buffer)
            .context("Cannot read disk source")?;
        if bytes_read == 0 {
            return Ok(());
        }
        let mut sent = 0;
        while sent < bytes_read {
            let count = stream
                .send(&buffer[sent..bytes_read])
                .context("Cannot send disk data")?;
            ensure!(count > 0, "Disk upload stream stopped accepting bytes");
            sent += count;
        }
    }
}

/// Grow the volume at `path` in the storage pool to `size_gb` GiB.
///
/// # Errors
///
/// Fails when `size_gb` is 0 or not larger than the volume, or libvirt cannot find, refresh or
/// resize it.
#[instrument(level = "debug", skip(conn))]
pub fn resize_disk(conn: &Connect, path: &str, size_gb: u16) -> Result<()> {
    ensure!(size_gb > 0, "Disk size must be greater than 0 GB");

    let capacity_bytes = gib_to_bytes(size_gb);
    let pool = StoragePool::lookup_by_name(conn, SHERPA_STORAGE_POOL)
        .with_context(|| format!("Cannot find storage pool '{SHERPA_STORAGE_POOL}'"))?;
    let vol = StorageVol::lookup_by_path(conn, path)
        .with_context(|| format!("Volume not found at path: {path}"))?;

    let info = vol
        .get_info()
        .with_context(|| format!("Failed to get volume info for: {path}"))?;

    ensure!(
        capacity_bytes > info.capacity,
        "Requested size {size_gb}G ({capacity_bytes} bytes) must be larger than current size ({} bytes) for: {path}",
        info.capacity
    );

    refresh_pool(&pool).context("Failed to refresh storage pool before resize")?;

    vol.resize(capacity_bytes, 0)
        .with_context(|| format!("Failed to resize volume '{path}' to {size_gb}G"))?;

    Ok(())
}

/// Delete the volume `disk_name` from the storage pool.
///
/// # Errors
///
/// Fails when the pool or the volume cannot be found, or libvirt cannot delete the volume.
#[instrument(level = "debug", skip(conn))]
pub fn delete_disk(conn: &Connect, disk_name: &str) -> Result<()> {
    let pool = StoragePool::lookup_by_name(conn, SHERPA_STORAGE_POOL)
        .with_context(|| format!("Cannot find storage pool '{SHERPA_STORAGE_POOL}'"))?;
    let vol = StorageVol::lookup_by_name(&pool, disk_name)
        .with_context(|| format!("Cannot find volume '{disk_name}'"))?;
    vol.delete(0)
        .with_context(|| format!("Cannot delete volume '{disk_name}'"))?;
    Ok(())
}

/// Define a persistent virtual machine from `xml` and start it. A VM that fails to start stays
/// defined; destroy finds it by name and removes it.
///
/// # Errors
///
/// Fails when libvirt cannot define or start the VM.
#[instrument(level = "debug", skip(conn, xml))]
pub fn create_vm(conn: &Connect, xml: &str) -> Result<Domain> {
    let domain = Domain::define_xml(conn, xml).context("Cannot define VM")?;
    domain.create().context("Cannot start VM")?;
    Ok(domain)
}

/// A virtual machine's management IP address: the first address of its first interface.
///
/// # Errors
///
/// Fails when the VM cannot be found or libvirt cannot read its interface addresses.
#[instrument(level = "debug", skip(conn))]
pub fn get_mgmt_ip(conn: &Connect, vm_name: &str) -> Result<Option<String>> {
    let domain = Domain::lookup_by_name(conn, vm_name)
        .with_context(|| format!("Cannot find VM '{vm_name}'"))?;
    let interfaces = domain
        .interface_addresses(0, 0)
        .with_context(|| format!("Cannot read the addresses of VM '{vm_name}'"))?;
    Ok(interfaces
        .first()
        .and_then(|interface| interface.addrs.first())
        .map(|ip| ip.addr.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copied_volumes_are_private_except_raw_qemu_arguments() {
        assert_eq!(volume_format_and_mode("qcow2").unwrap(), ("qcow2", "0600"));
        assert_eq!(volume_format_and_mode("ISO").unwrap(), ("raw", "0600"));
        assert_eq!(volume_format_and_mode("elf").unwrap(), ("raw", "0600"));
        // QEMU opens Flatcar's Ignition file itself, so libvirt never hands it over.
        assert_eq!(volume_format_and_mode("ign").unwrap(), ("raw", "0644"));
        assert!(volume_format_and_mode("exe").is_err());
    }

    #[test]
    fn copied_volume_xml_carries_the_mode() {
        let xml = CopyVolume {
            name: "dev-lab.iso",
            destination: "/pool/dev-lab.iso",
            format: "raw",
            mode: "0600",
        }
        .render()
        .unwrap();
        assert!(xml.contains("<permissions><mode>0600</mode></permissions>"));
    }
}
