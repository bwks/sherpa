use std::env;
use std::fs;
#[cfg(unix)]
use std::fs::{DirBuilder, File, OpenOptions};
#[cfg(unix)]
use std::io::{self, BufReader, BufWriter, ErrorKind, Write};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
#[cfg(unix)]
use std::process::Command;

#[cfg(unix)]
use anyhow::bail;
use anyhow::{Context, Result};
#[cfg(unix)]
use tracing::instrument;

use crate::data::{KindSettings, UnikernelBootMode};
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

/// Write `content` to `path` with permission bits `mode`, replacing any file there.
///
/// The file never exists with wider permissions: it is written to a temporary file created
/// with `mode` in the same folder, then renamed over `path`. Missing parent folders are created
/// private (0700); existing folders keep their permissions.
///
/// # Errors
///
/// Fails when a parent folder cannot be created, or the file cannot be written or renamed.
#[cfg(unix)]
#[instrument(level = "debug", skip_all, fields(path = %path.display(), mode = format!("{mode:o}")))]
pub fn write_file_with_permissions(path: &Path, content: &[u8], mode: u32) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("No parent directory for: {}", path.display()))?;
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)
        .with_context(|| format!("Failed to create directory: {}", parent.display()))?;

    let file_name = path
        .file_name()
        .with_context(|| format!("No file name in: {}", path.display()))?;
    let temp_path = parent.join(format!(".{}.tmp", file_name.to_string_lossy()));
    // A leftover from an interrupted write would make `create_new` fail.
    match fs::remove_file(&temp_path) {
        Ok(()) => tracing::debug!(path = %temp_path.display(), "Removed a stale temporary file"),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            tracing::trace!(path = %temp_path.display(), "No stale temporary file");
        }
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to remove {}", temp_path.display()));
        }
    }

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&temp_path)
        .with_context(|| format!("Failed to create file: {}", temp_path.display()))?;
    // The umask may have narrowed `mode`; set it exactly.
    file.set_permissions(fs::Permissions::from_mode(mode))
        .with_context(|| format!("Failed to set permissions on {}", temp_path.display()))?;
    file.write_all(content)
        .and_then(|()| file.sync_all())
        .with_context(|| format!("Failed to write to file: {}", temp_path.display()))?;
    fs::rename(&temp_path, path)
        .with_context(|| format!("Failed to replace file: {}", path.display()))
}

/// Create the folder `path` with permission bits `mode`, whatever the umask. Missing folders
/// above it are created private (0700); an existing `path` is set to `mode`, and its contents
/// are left alone.
///
/// # Errors
///
/// Fails when a folder cannot be created or its mode cannot be set.
#[cfg(unix)]
#[instrument(level = "debug", fields(path = %path.display(), mode = format!("{mode:o}")))]
pub fn create_dir_with_permissions(path: &Path, mode: u32) -> Result<()> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .with_context(|| format!("Failed to create directory: {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("Failed to set permissions on {}", path.display()))
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

/// Copy the file `src` to `dst`, with permission bits `mode` whatever the umask, replacing any
/// file at `dst` and its mode.
///
/// # Errors
///
/// Fails when `src` cannot be read, or `dst` written or its mode set.
#[cfg(unix)]
#[instrument(level = "debug", fields(mode = format!("{mode:o}")))]
pub fn copy_file_with_permissions(src: &str, dst: &str, mode: u32) -> Result<()> {
    let source = File::open(src).with_context(|| format!("Error loading src file: {src}"))?;
    let destination = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(dst)
        .with_context(|| format!("Error creating dst file: {dst}"))?;
    // The umask may have narrowed `mode`, and a file that was there keeps its own; set it.
    destination
        .set_permissions(fs::Permissions::from_mode(mode))
        .with_context(|| format!("Failed to set permissions on {dst}"))?;

    let mut reader = BufReader::new(source);
    let mut writer = BufWriter::new(destination);

    io::copy(&mut reader, &mut writer)
        .with_context(|| format!("Error copying file: {src} -> {dst}"))?;

    writer
        .flush()
        .with_context(|| "Error flushing writer contents")?; // Ensures all buffered contents are written to the file

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

/// Create a config archive using the `tar` command, with permission bits `mode` whatever the
/// umask.
///
/// `tar` must be installed on the system.
///
/// # Errors
///
/// Fails when `tar` cannot be run or exits with an error, or the archive's mode cannot be set.
#[cfg(unix)]
pub fn create_config_archive(src_path: &str, dst_path: &str, mode: u32) -> Result<()> {
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
    fs::set_permissions(dst_path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("Failed to set permissions on {dst_path}"))?;
    tracing::debug!(source = %src_path, archive = %dst_path, "Archive created");
    Ok(())
}

/// The file a disk-based image keeps in its version folder, by its kind's settings; `None` for
/// a container image, which Docker keeps.
///
/// - VMs: `virtioa.qcow2`
/// - Unikernel `DirectKernel`: `kernel.elf`
/// - Unikernel `DiskBoot`: `disk.qcow2`
#[must_use]
pub fn image_filename(settings: &KindSettings) -> Option<&'static str> {
    match settings {
        KindSettings::VirtualMachine(_) => Some(VM_DISK_FILENAME),
        KindSettings::Unikernel(unikernel) => Some(match unikernel.boot_mode {
            UnikernelBootMode::DirectKernel => UNIKERNEL_KERNEL_FILENAME,
            UnikernelBootMode::DiskBoot => UNIKERNEL_DISK_FILENAME,
        }),
        KindSettings::Container(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use anyhow::Result;
    use tempfile::TempDir;

    use super::*;

    #[cfg(unix)]
    fn mode_of(path: &Path) -> u32 {
        fs::metadata(path).expect("metadata").permissions().mode() & 0o777
    }

    #[cfg(unix)]
    #[test]
    fn config_privacy_010_writing_a_file_leaves_its_folder_alone() {
        let dir = TempDir::new().expect("tempdir");
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).expect("chmod");

        write_file_with_permissions(&dir.path().join("key.pem"), b"secret", 0o600).expect("write");

        assert_eq!(mode_of(dir.path()), 0o755);
    }

    #[cfg(unix)]
    #[test]
    fn config_privacy_009_a_rewritten_file_gets_the_new_mode_and_contents() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("key.pem");
        fs::write(&path, b"old").expect("seed");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("chmod");

        write_file_with_permissions(&path, b"secret", 0o600).expect("write");

        assert_eq!(mode_of(&path), 0o600);
        assert_eq!(fs::read(&path).expect("read"), b"secret");
        let leftovers = fs::read_dir(dir.path()).expect("read dir").count();
        assert_eq!(leftovers, 1, "a temporary file was left behind");
    }

    #[cfg(unix)]
    #[test]
    fn config_privacy_011_folders_made_on_the_way_are_private_and_a_new_one_gets_its_mode() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("a/b/cert.pem");

        write_file_with_permissions(&path, b"cert", 0o644).expect("write");

        assert_eq!(mode_of(&path), 0o644);
        assert_eq!(mode_of(&dir.path().join("a")), 0o700);
        assert_eq!(mode_of(&dir.path().join("a/b")), 0o700);

        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("a/b/tftp");

        create_dir_with_permissions(&path, 0o705).expect("create");

        assert_eq!(mode_of(&path), 0o705);
        assert_eq!(mode_of(&dir.path().join("a")), 0o700);
        assert_eq!(mode_of(&dir.path().join("a/b")), 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn config_privacy_012_an_existing_folder_is_set_to_the_mode_and_keeps_its_contents() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("node");
        fs::create_dir(&path).expect("mkdir");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o2755)).expect("chmod");
        fs::write(path.join("config"), b"config").expect("seed");

        create_dir_with_permissions(&path, 0o700).expect("create");

        let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o7777;
        assert_eq!(
            mode, 0o700,
            "the setgid bit an inherited folder had is cleared too"
        );
        assert_eq!(fs::read(path.join("config")).expect("read"), b"config");
    }

    #[cfg(unix)]
    #[test]
    fn server_file_modes_015_doctor_boxes_sets_folders_to_775_and_files_to_660() -> Result<()> {
        let dir = TempDir::new()?;
        let file = dir.path().join("perms.txt");
        fs::write(&file, "data")?;
        fix_permissions_recursive(file.to_str().unwrap())?;
        let mode = fs::metadata(&file)?.permissions().mode() & 0o777;
        assert_eq!(mode, 0o660);

        let sub = dir.path().join("subdir");
        fs::create_dir(&sub)?;
        let nested = sub.join("file.txt");
        fs::write(&nested, "data")?;
        fix_permissions_recursive(sub.to_str().unwrap())?;
        let dir_mode = fs::metadata(&sub)?.permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o775);
        let file_mode = fs::metadata(&nested)?.permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o660);
        Ok(())
    }
}
