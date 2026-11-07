use std::env;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
#[cfg(unix)]
use std::process::Command;

#[cfg(unix)]
use anyhow::bail;
use anyhow::{Context, Result};

use crate::data::{NodeKind, UnikernelBootMode};
use crate::konst::{UNIKERNEL_DISK_FILENAME, UNIKERNEL_KERNEL_FILENAME, VM_DISK_FILENAME};

/// Convert a `Path` to an owned `String`, using lossy UTF-8 conversion.
#[must_use]
pub fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

/// Load a file
///
/// # Errors
///
/// Fails when the file cannot be read as UTF-8 text.
pub fn load_file(file_path: &str) -> Result<String> {
    let path = expand_path(file_path);
    let file_contents =
        fs::read_to_string(&path).with_context(|| format!("Error loading file: {file_path}"))?;
    Ok(file_contents)
}

/// Get the current working directory path
///
/// # Errors
///
/// Fails when the working directory is unknown or its path is not UTF-8.
pub fn get_cwd() -> Result<String> {
    let path = env::current_dir()?;
    Ok(path.display().to_string())
}

/// Create a directory
///
/// # Errors
///
/// Fails when the directory cannot be created.
pub fn create_dir(dir_path: &str) -> Result<()> {
    fs::create_dir_all(dir_path)
        .with_context(|| format!("Error creating directory: {dir_path}"))?;
    tracing::debug!(path = %dir_path, "Directory created");
    Ok(())
}

/// Create a file
///
/// # Errors
///
/// Fails when the file cannot be written.
pub fn create_file(file_path: &str, contents: String) -> Result<()> {
    let size = contents.len();
    fs::write(file_path, contents).with_context(|| format!("Error creating file: {file_path}"))?;
    tracing::debug!(path = %file_path, size = size, "File created");
    Ok(())
}

/// Check for file existence
#[must_use]
pub fn file_exists(file_path: &str) -> bool {
    let path = Path::new(file_path);
    path.exists() && path.is_file()
}

/// Check for directory existence
#[must_use]
pub fn dir_exists(dir_path: &str) -> bool {
    let path = Path::new(dir_path);
    path.exists() && path.is_dir()
}

/// Expand a path if it's passed with ~
#[must_use]
pub fn expand_path(path: &str) -> String {
    let expanded_path = shellexpand::tilde(path);
    let full_path = Path::new(expanded_path.as_ref());
    full_path.display().to_string()
}

/// Recursively delete a directory
///
/// # Errors
///
/// Fails when the directory exists but cannot be removed.
pub fn delete_dirs(dir_path: &str) -> Result<()> {
    if dir_exists(dir_path) {
        fs::remove_dir_all(dir_path)?;
        tracing::debug!(path = %dir_path, "Directory deleted");
    }
    Ok(())
}

/// Copy a file from a source to a destination.
/// This will overwrite the destination file if it exists.
///
/// # Errors
///
/// Fails when `src` cannot be read or `dst` written.
pub fn copy_file(src: &str, dst: &str) -> Result<()> {
    let source = File::open(src).with_context(|| format!("Error loading src file: {src}"))?;
    let destination =
        File::create(dst).with_context(|| format!("Error creating dst file: {dst}"))?;

    let mut reader = BufReader::new(source);
    let mut writer = BufWriter::new(destination);

    io::copy(&mut reader, &mut writer)
        .with_context(|| format!("Error copying file: {src} -> {dst}"))?;

    writer
        .flush()
        .with_context(|| "Error flushing writer contents")?; // Ensures all buffered contents are written to the file

    Ok(())
}

/// Set Unix file permissions on a single file.
///
/// # Errors
///
/// Fails when the mode cannot be set.
#[cfg(unix)]
#[tracing::instrument(level = "debug")]
pub fn set_file_permissions(path: &str, mode: u32) -> Result<()> {
    let perms = std::fs::Permissions::from_mode(mode);
    fs::set_permissions(path, perms).context(format!("Failed to set permissions on {path}"))?;
    Ok(())
}

/// Set permissions for all files in a folder subtree.
/// Sets files to read-write and removes executable bit for users/groups.
///
/// # Errors
///
/// Fails when a mode cannot be set or a directory cannot be read.
#[cfg(unix)]
pub fn fix_permissions_recursive(path: &str) -> Result<()> {
    let path = expand_path(path);

    let metadata = fs::metadata(&path)?;
    let mut perms = metadata.permissions();

    if metadata.is_dir() {
        // Set directory permissions
        perms.set_mode(0o775);
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

/// Copy a file to a virtual DOS disk image using the `mcopy` command.
///
/// `mcopy` must be installed on the system.
///
/// # Errors
///
/// Fails when `mcopy` cannot be run or exits with an error.
#[cfg(unix)]
pub fn copy_to_dos_image(src_file: &str, dst_image: &str, dst_dir: &str) -> Result<()> {
    let status = Command::new("mcopy")
        .args(["-i", dst_image, src_file, &format!("::{dst_dir}")])
        .status()?;
    if !status.success() {
        bail!(
            "mcopy failed (exit {}): copying {} to image {}",
            status.code().unwrap_or(-1),
            src_file,
            dst_image
        );
    }
    tracing::debug!(source = %src_file, image = %dst_image, directory = %dst_dir, "File copied to DOS image");

    Ok(())
}

/// Copy a file to a virtual EXT4 disk image using the `e2cp` command.
///
/// `e2cp` must be installed on the system.
///
/// # Errors
///
/// Fails when `e2cp` cannot be run or exits with an error.
#[cfg(unix)]
pub fn copy_to_ext4_image(src_files: &[&str], dst_image: &str, dst_dir: &str) -> Result<()> {
    let dst = format!("{dst_image}:{dst_dir}");
    let status = Command::new("e2cp").args(src_files).arg(&dst).status()?;
    if !status.success() {
        bail!(
            "e2cp failed (exit {}): copying to image {}",
            status.code().unwrap_or(-1),
            dst_image
        );
    }
    tracing::debug!(image = %dst_image, directory = %dst_dir, file_count = src_files.len(), "Files copied to EXT4 image");

    Ok(())
}

/// Create a config archive using the `tar` command.
///
/// `tar` must be installed on the system.
///
/// # Errors
///
/// Fails when `tar` cannot be run or exits with an error.
#[cfg(unix)]
pub fn create_config_archive(src_path: &str, dst_path: &str) -> Result<()> {
    let path = Path::new(src_path);
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("No parent directory for: {src_path}"))?;
    let filename = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("No filename for: {src_path}"))?;

    let status = Command::new("tar")
        .args([
            "czf",
            dst_path,
            "-C",
            &dir.to_string_lossy(),
            &filename.to_string_lossy(),
        ])
        .status()?;
    if !status.success() {
        bail!(
            "tar failed (exit {}): creating archive {} from {}",
            status.code().unwrap_or(-1),
            dst_path,
            src_path
        );
    }
    tracing::debug!(source = %src_path, archive = %dst_path, "Archive created");
    Ok(())
}

/// Return the expected image filename for a given node kind and boot mode.
///
/// - VMs: `virtioa.qcow2`
/// - Unikernel `DirectKernel`: `kernel.elf`
/// - Unikernel `DiskBoot`: `disk.qcow2`
#[must_use]
pub fn image_filename(kind: &NodeKind, boot_mode: Option<&UnikernelBootMode>) -> &'static str {
    match kind {
        NodeKind::Unikernel => match boot_mode {
            Some(UnikernelBootMode::DiskBoot) => UNIKERNEL_DISK_FILENAME,
            _ => UNIKERNEL_KERNEL_FILENAME,
        },
        _ => VM_DISK_FILENAME,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Result;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_path_to_string() {
        let path = std::path::Path::new("/tmp/test.txt");
        assert_eq!(path_to_string(path), "/tmp/test.txt");
    }

    #[test]
    fn test_get_cwd_returns_a_path() {
        let cwd = get_cwd().unwrap();
        assert_ne!(cwd, "");
    }

    #[test]
    fn test_expand_path_no_tilde() {
        let result = expand_path("/tmp/foo/bar");
        assert_eq!(result, "/tmp/foo/bar");
    }

    #[test]
    fn test_expand_path_with_tilde() {
        let result = expand_path("~/something");
        // After expansion ~ should be replaced with the home directory
        assert!(!result.starts_with('~'));
        assert!(result.ends_with("something"));
    }

    #[test]
    fn test_file_exists_false_for_nonexistent() {
        assert!(!file_exists("/tmp/sherpa_nonexistent_file_12345.txt"));
    }

    #[test]
    fn test_file_exists_true_after_create() -> Result<()> {
        let dir = TempDir::new()?;
        let path = dir.path().join("exists.txt");
        fs::write(&path, "hello")?;
        assert!(file_exists(path.to_str().unwrap()));
        Ok(())
    }

    #[test]
    fn test_file_exists_false_for_directory() -> Result<()> {
        let dir = TempDir::new()?;
        assert!(!file_exists(dir.path().to_str().unwrap()));
        Ok(())
    }

    #[test]
    fn test_dir_exists_false_for_nonexistent() {
        assert!(!dir_exists("/tmp/sherpa_nonexistent_dir_12345"));
    }

    #[test]
    fn test_dir_exists_true_for_existing() -> Result<()> {
        let dir = TempDir::new()?;
        assert!(dir_exists(dir.path().to_str().unwrap()));
        Ok(())
    }

    #[test]
    fn test_dir_exists_false_for_file() -> Result<()> {
        let dir = TempDir::new()?;
        let path = dir.path().join("file.txt");
        fs::write(&path, "data")?;
        assert!(!dir_exists(path.to_str().unwrap()));
        Ok(())
    }

    #[test]
    fn test_create_dir_and_verify() -> Result<()> {
        let dir = TempDir::new()?;
        let new_dir = dir.path().join("nested/deep");
        create_dir(new_dir.to_str().unwrap())?;
        assert!(new_dir.is_dir());
        Ok(())
    }

    #[test]
    fn test_create_file_and_load_file() -> Result<()> {
        let dir = TempDir::new()?;
        let path = dir.path().join("hello.txt");
        create_file(path.to_str().unwrap(), "hello world".to_string())?;
        let content = load_file(path.to_str().unwrap())?;
        assert_eq!(content, "hello world");
        Ok(())
    }

    #[test]
    fn test_load_file_nonexistent_returns_err() {
        assert!(load_file("/tmp/sherpa_nonexistent_99999.txt").is_err());
    }

    #[test]
    fn test_delete_dirs_removes_directory() -> Result<()> {
        let dir = TempDir::new()?;
        let sub = dir.path().join("subdir");
        fs::create_dir(&sub)?;
        fs::write(sub.join("file.txt"), "data")?;
        assert!(sub.is_dir());
        delete_dirs(sub.to_str().unwrap())?;
        assert!(!sub.exists());
        Ok(())
    }

    #[test]
    fn test_delete_dirs_nonexistent_is_ok() {
        assert!(delete_dirs("/tmp/sherpa_nonexistent_dir_delete").is_ok());
    }

    #[test]
    fn test_copy_file_copies_contents() -> Result<()> {
        let dir = TempDir::new()?;
        let src = dir.path().join("src.txt");
        let dst = dir.path().join("dst.txt");
        fs::write(&src, "copy me")?;
        copy_file(src.to_str().unwrap(), dst.to_str().unwrap())?;
        assert_eq!(fs::read_to_string(&dst)?, "copy me");
        Ok(())
    }

    #[test]
    fn test_copy_file_nonexistent_src_returns_err() {
        assert!(copy_file("/tmp/sherpa_no_src.txt", "/tmp/sherpa_dst.txt").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn test_fix_permissions_recursive_file() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new()?;
        let file = dir.path().join("perms.txt");
        fs::write(&file, "data")?;
        fix_permissions_recursive(file.to_str().unwrap())?;
        let mode = fs::metadata(&file)?.permissions().mode() & 0o777;
        assert_eq!(mode, 0o660);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn test_fix_permissions_recursive_dir() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new()?;
        let sub = dir.path().join("subdir");
        fs::create_dir(&sub)?;
        let file = sub.join("file.txt");
        fs::write(&file, "data")?;
        fix_permissions_recursive(sub.to_str().unwrap())?;
        let dir_mode = fs::metadata(&sub)?.permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o775);
        let file_mode = fs::metadata(&file)?.permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o660);
        Ok(())
    }
}
