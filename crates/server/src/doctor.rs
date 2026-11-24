use std::ffi::OsStr;
use std::fs::{self, Permissions};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};
use tracing::instrument;

use shared::konst::SHERPA_IMAGES_PATH;
use shared::util::term_msg_surround;

use crate::konst::SHERPA_BLANK_DISK_DIR;

/// Every folder under the images folder: `sherpa` and its group, who import images, may write.
const BOX_FOLDER_MODE: u32 = 0o775;
/// Any other file under the images folder: `sherpa` and its group may read and replace it.
const BOX_FILE_MODE: u32 = 0o660;
/// A blank disk template: only sherpad, as `sherpa`, its owner, reads it, to copy it.
const BLANK_DISK_FILE_MODE: u32 = 0o400;

/// Fix the server's local state: with `boxes`, the permissions of the base box images.
///
/// # Errors
///
/// Fails when the image permissions cannot be changed.
pub fn doctor(boxes: bool) -> Result<()> {
    if boxes {
        term_msg_surround("Fixing base box permissions");
        fix_box_permissions(Path::new(SHERPA_IMAGES_PATH), BOX_FILE_MODE)?;
    }
    Ok(())
}

/// Set `path` and everything under it to the box modes: folders `BOX_FOLDER_MODE`, files
/// `file_mode`, and the files of the blank disk folder `BLANK_DISK_FILE_MODE`.
#[instrument(level = "debug", fields(path = %path.display()))]
fn fix_box_permissions(path: &Path, file_mode: u32) -> Result<()> {
    let metadata =
        fs::metadata(path).with_context(|| format!("Failed to read {}", path.display()))?;
    if metadata.is_dir() {
        fs::set_permissions(path, Permissions::from_mode(BOX_FOLDER_MODE))
            .with_context(|| format!("Failed to set the mode of {}", path.display()))?;
        let file_mode = if path.file_name() == Some(OsStr::new(SHERPA_BLANK_DISK_DIR)) {
            BLANK_DISK_FILE_MODE
        } else {
            file_mode
        };
        for entry in
            fs::read_dir(path).with_context(|| format!("Failed to read {}", path.display()))?
        {
            fix_box_permissions(&entry?.path(), file_mode)?;
        }
    } else {
        fs::set_permissions(path, Permissions::from_mode(file_mode))
            .with_context(|| format!("Failed to set the mode of {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o7777
    }

    #[test]
    fn server_file_modes_015_doctor_boxes_sets_folders_775_blank_disks_400_and_other_files_660() {
        let images = TempDir::new().unwrap();
        let version = images.path().join("alpine_linux/3.22");
        fs::create_dir_all(&version).unwrap();
        let image = version.join("virtioa.qcow2");
        fs::write(&image, "image").unwrap();
        fs::set_permissions(&image, Permissions::from_mode(0o644)).unwrap();
        let blank_disks = images.path().join(SHERPA_BLANK_DISK_DIR);
        fs::create_dir(&blank_disks).unwrap();
        fs::set_permissions(&blank_disks, Permissions::from_mode(0o700)).unwrap();
        let blank_disk = blank_disks.join("fat32.img");
        fs::write(&blank_disk, "disk").unwrap();
        fs::set_permissions(&blank_disk, Permissions::from_mode(0o660)).unwrap();

        fix_box_permissions(images.path(), BOX_FILE_MODE).unwrap();

        for folder in [images.path(), &version, &blank_disks] {
            assert_eq!(mode(folder), 0o775, "{}", folder.display());
        }
        assert_eq!(mode(&image), 0o660);
        assert_eq!(mode(&blank_disk), 0o400);
        assert_eq!(fs::read(&blank_disk).unwrap(), b"disk");
    }
}
