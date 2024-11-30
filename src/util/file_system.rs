use std::env;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use anyhow::Result;

/// Get the current working directory path
pub fn get_cwd() -> Result<String> {
    let path = env::current_dir()?;
    Ok(path.display().to_string())
}

/// Create a directory, expanding ~ if it's passed
pub fn create_dir(dir_path: &str) -> Result<()> {
    fs::create_dir_all(dir_path)?;
    println!("Path created: {dir_path}");

    Ok(())
}

/// Create a file, expanding ~ if it's passed
pub fn create_file(file_path: &str, contents: String) -> Result<()> {
    fs::write(file_path, contents)?;
    println!("File created: {file_path}");

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

/// Copy a file from a source to a destination.
/// This will overwrite the destination file if it exists.
pub fn copy_file(src: &str, dst: &str) -> Result<()> {
    let source = File::open(src)?;
    let destination = File::create(dst)?;

    let mut reader = BufReader::new(source);
    let mut writer = BufWriter::new(destination);

    io::copy(&mut reader, &mut writer)?;

    writer.flush()?; // Ensures all buffered contents are written to the file

    Ok(())
}

/// Set permissions for all files in a folder subtree.
/// Sets files to read-write and removes executable bit for users/groups.
pub fn fix_permissions_recursive(path: &str) -> Result<()> {
    let path = expand_path(path);

    let metadata = fs::metadata(&path)?;
    let mut perms = metadata.permissions();

    if metadata.is_dir() {
        // Set directory permissions
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;

        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            fix_permissions_recursive(
                entry
                    .path()
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("Error updating read-only permissions"))?,
            )?;
        }
    } else {
        // Set file permissions
        perms.set_mode(0o660);
        fs::set_permissions(path, perms)?;
    }

    Ok(())
}

/// Create a ZTP ISO file.
/// This wraps the `genisoimage` command to create a ztp ISO file
/// from a directory of source files.
///
/// `genisoimage` must be installed on the system.
///
/// DOGWATER: Implement this functionality in pure Rust.
pub fn create_ztp_iso(iso_dst: &str, src_dir: String) -> Result<()> {
    // Create ISO using genisoimage
    Command::new("genisoimage")
        .args([
            "-output",
            iso_dst,
            "-volid",
            "cidata",
            "-joliet",
            "-rock",
            "--input-charset",
            "utf-8",
            &src_dir,
        ])
        .status()?;
    println!("ISO created successfully: {iso_dst}");

    Ok(())
}

/// Copy a file to a virtual DOS disk image using the `mcopy` command.
///
/// `mcopy` must be installed on the system.
pub fn copy_to_dos_image(src_file: &str, dst_image: &str, dst_dir: &str) -> Result<()> {
    Command::new("mcopy")
        .args(["-i", dst_image, src_file, &format!("::{dst_dir}")])
        .status()?;
    println!("File copied to DOS image: {dst_image}");

    Ok(())
}

/// Convert an ISO file to a Qcow2 disk image.
pub fn _convert_iso_qcow2(src_iso: &str, dst_disk: &str) -> Result<()> {
    Command::new("qemu-img")
        .args(["convert", "-O", "qcow2", src_iso, dst_disk])
        .status()?;
    Ok(())
}
