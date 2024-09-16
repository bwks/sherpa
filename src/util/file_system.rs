use std::fs;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::Result;

use virt::connect::Connect;
use virt::storage_pool::StoragePool;
use virt::storage_vol::StorageVol;
use virt::stream::Stream;

use crate::core::konst::STORAGE_POOL;

/// Create a directory, expanding ~ if it's passed
pub fn create_dir(dir_path: &str) -> Result<()> {
    fs::create_dir_all(&dir_path)?;
    println!("Directory path created successfully: {dir_path}");

    Ok(())
}

/// Create a file, expanding ~ if it's passed
pub fn create_file(file_path: &str, contents: String) -> Result<()> {
    fs::write(file_path, contents)?;
    println!("File created successfully: {file_path}");

    Ok(())
}

/// Check for file existence, expanding ~ if it's passed
pub fn file_exists(file_path: &str) -> bool {
    let path = Path::new(file_path);
    path.exists() && path.is_file()
}

/// Check for directory existence, expanding ~ if it's passed
pub fn dir_exists(dir_path: &str) -> bool {
    let path = Path::new(dir_path);
    path.exists() && path.is_dir()
}

/// Expand a path if it's passed with ~
pub fn expand_path(path: &str) -> String {
    let expanded_path = shellexpand::tilde(path);
    let full_path = Path::new(expanded_path.as_ref());
    full_path.display().to_string()
}

/// Clone a disk image using libvirt
pub fn clone_disk(conn: &Connect, src_path: &str, dst_path: &str) -> Result<()> {
    let pool = StoragePool::lookup_by_name(conn, STORAGE_POOL)?;

    let file_path = Path::new(dst_path);
    let file_name = file_path.file_name().unwrap().to_str().unwrap();

    let file_extension = file_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");

    let format_type = match file_extension.to_lowercase().as_str() {
        "iso" => "raw",
        _ => "qcow2",
    };

    let vol_xml = format!(
        r#"<volume>
            <name>{file_name}</name>
            <allocation>0</allocation>
            <capacity>0</capacity>
            <target>
                <path>{dst_path}</path>
                <format type='{format_type}'/>
            </target>
        </volume>"#
    );

    // Create the new volume using the Connect struct
    let new_vol = StorageVol::create_xml(&pool, &vol_xml, 0)?;

    // Open the source file
    let mut src_file = File::open(&src_path)?;

    // Get the file size
    let file_size = src_file.seek(SeekFrom::End(0))?;
    src_file.seek(SeekFrom::Start(0))?;

    // Create a new stream
    let stream = Stream::new(conn, 0)?;

    // Start the upload
    new_vol.upload(&stream, 0, file_size, 0)?;

    // Define chunk size (e.g., 10 MB)
    const CHUNK_SIZE: usize = 10 * 1024 * 1024;
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

/// Delete a volume
pub fn delete_disk(conn: &Connect, vol_name: &str) -> Result<()> {
    let pool = StoragePool::lookup_by_name(conn, STORAGE_POOL)?;
    let vol = StorageVol::lookup_by_name(&pool, vol_name)?;
    vol.delete(0)?;
    Ok(())
}
