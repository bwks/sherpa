use std::env;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Write};
use std::os::unix::fs::{symlink, PermissionsExt};
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

/// Check if a file is between a certain size.
/// Currently only supports file sizes up to 5GB
pub fn check_file_size(path: &str) -> Result<u8> {
    let expanded_path = shellexpand::tilde(path);
    let metadata = fs::metadata(Path::new(expanded_path.as_ref()))?;
    let size_in_bytes = metadata.len();
    let size_in_gb = size_in_bytes as f64 / (1024.0 * 1024.0 * 1024.0);

    let result = match size_in_gb {
        0.1..1.0 => 1,
        1.0..2.0 => 2,
        2.0..3.0 => 3,
        3.0..4.0 => 4,
        4.0..5.0 => 5,
        _ => 0,
    };

    Ok(result as u8)
}

/// Delete a file, expanding ~ if it's passed
#[allow(dead_code)]
pub fn delete_file(file_path: &str) -> Result<()> {
    let path = expand_path(file_path);
    if file_exists(&path) {
        std::fs::remove_file(&path)?;
        println!("File deleted: {path}");
    }
    Ok(())
}

/// Recursively delete a directory
pub fn delete_dirs(dir_path: &str) -> Result<()> {
    if dir_exists(dir_path) {
        fs::remove_dir_all(dir_path)?;
        println!("Deleted path: {dir_path}");
    }
    Ok(())
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

/// Create a symbolic link from source to target path
/// Will expand ~ in paths if present
pub fn create_symlink(src: &str, target: &str) -> Result<()> {
    let expanded_src = expand_path(src);
    let expanded_target = expand_path(target);

    // Remove target if it exists
    if file_exists(&expanded_target) {
        std::fs::remove_file(&expanded_target)?;
    }

    symlink(&expanded_src, &expanded_target)?;
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

/// Copy a file to a virtual EXT4 disk image using the `e2cp` command.
///
/// `e2cp` must be installed on the system.
pub fn copy_to_ext4_image(src_file: &str, dst_image: &str, dst_dir: &str) -> Result<()> {
    Command::new("e2cp")
        .args([src_file, &format!("{dst_image}:{dst_dir}")])
        .status()?;
    println!("File copied to EXT4 image: {dst_image}");

    Ok(())
}

/// Create a config archive using the `tar` command.
///
/// `tar` must be installed on the system.
pub fn create_config_archive(src_path: &str, dst_path: &str) -> Result<()> {
    Command::new("tar")
        .args(["cvzf", dst_path, src_path])
        .status()?;
    println!("Archive created: {dst_path}");
    Ok(())
}

/// Convert an ISO file to a Qcow2 disk image.
pub fn _convert_iso_qcow2(src_iso: &str, dst_disk: &str) -> Result<()> {
    Command::new("qemu-img")
        .args(["convert", "-O", "qcow2", src_iso, dst_disk])
        .status()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self};
    use tempfile::TempDir;

    #[test]
    fn test_create_symlink() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let source_path = temp_dir.path().join("source.txt");
        let target_path = temp_dir.path().join("target.txt");

        // Create source file
        fs::write(&source_path, "test content")?;

        // Create symlink
        create_symlink(source_path.to_str().unwrap(), target_path.to_str().unwrap())?;

        // Verify symlink exists and points to source
        assert!(target_path.exists());
        assert!(target_path.is_symlink());
        assert_eq!(
            fs::read_to_string(&target_path)?,
            fs::read_to_string(&source_path)?
        );

        Ok(())
    }

    #[test]
    fn test_create_symlink_existing_target() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let source_path = temp_dir.path().join("source.txt");
        let target_path = temp_dir.path().join("target.txt");

        // Create source and initial target
        fs::write(&source_path, "source content")?;
        fs::write(&target_path, "target content")?;

        // Create symlink (should overwrite target)
        create_symlink(source_path.to_str().unwrap(), target_path.to_str().unwrap())?;

        assert!(target_path.is_symlink());
        assert_eq!(fs::read_to_string(&target_path)?, "source content");

        Ok(())
    }
}
