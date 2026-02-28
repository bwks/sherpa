use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::Result;

use virt::connect::Connect;
use virt::domain::Domain;
use virt::storage_pool::StoragePool;
use virt::storage_vol::StorageVol;
use virt::stream::Stream;

use shared::konst::SHERPA_STORAGE_POOL;

/// Clone a disk image.
pub fn clone_disk(conn: &Connect, src_path: &str, dst_path: &str) -> Result<()> {
    let pool = StoragePool::lookup_by_name(conn, SHERPA_STORAGE_POOL)?;

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
        "iso" => "raw",
        "json" => "raw",
        "ign" => "raw",
        "img" => "raw",
        "qcow2" => "qcow2",
        _ => {
            return Err(anyhow::anyhow!(
                "Unsupported file extension: {}",
                file_extension
            ));
        }
    };

    let vol_xml = format!(
        r#"<volume>
            <name>{file_name}</name>
            <allocation>0</allocation>
            <capacity>0</capacity>
            <target>
                <path>{dst_path}</path>
                <format type='{format_type}'/>
                <permissions>
                    <mode>0644</mode>
                </permissions>
            </target>
        </volume>"#
    );

    // Create the new volume using the Connect struct
    let new_vol = StorageVol::create_xml(&pool, &vol_xml, 0)?;

    // Open the source file
    let mut src_file = File::open(src_path)?;

    // Get the file size
    let file_size = src_file.seek(SeekFrom::End(0))?;
    src_file.seek(SeekFrom::Start(0))?;

    // Create a new stream
    let stream = Stream::new(conn, 0)?;

    // Start the upload
    new_vol.upload(&stream, 0, file_size, 0)?;

    // Define chunk size (e.g., 25 MB)
    const CHUNK_SIZE: usize = 25 * 1024 * 1024;
    let mut buffer = vec![0; CHUNK_SIZE];

    // Read and send data in chunks
    loop {
        let bytes_read = src_file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        stream.send(&buffer[..bytes_read])?;
    }

    // Finish the stream
    stream.finish()?;

    Ok(())
}

/// Delete a volume from the storage pool.
pub fn delete_disk(conn: &Connect, disk_name: &str) -> Result<()> {
    let pool = StoragePool::lookup_by_name(conn, SHERPA_STORAGE_POOL)?;
    let vol = StorageVol::lookup_by_name(&pool, disk_name)?;
    vol.delete(0)?;
    Ok(())
}

/// Create a virtual machine. This will define a persistent virtual machine.
pub fn create_vm(conn: &Connect, xml: &str) -> Result<Domain> {
    let domain = Domain::define_xml(conn, xml)?;
    domain.create()?;
    Ok(domain)
}

/// Get virtual machine's Management IP address
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
