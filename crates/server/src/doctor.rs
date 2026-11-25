use std::ffi::{CString, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use nix::dir::Dir;
use nix::errno::Errno;
use nix::fcntl::OFlag;
use nix::sys::stat::{FchmodatFlags, Mode, fchmod, fchmodat};
use tracing::instrument;

use shared::konst::SHERPA_IMAGES_PATH;
use shared::util::term_msg_surround;

use crate::konst::SHERPA_BLANK_DISK_DIR;

/// Every folder under the images folder: `sherpa` and its group, who import images, may write.
const BOX_FOLDER_MODE: Mode = Mode::from_bits_truncate(0o775);
/// Any other file under the images folder: `sherpa` and its group may read and replace it.
const BOX_FILE_MODE: Mode = Mode::from_bits_truncate(0o660);
/// A blank disk template: only sherpad, as `sherpa`, its owner, reads it, to copy it.
const BLANK_DISK_FILE_MODE: Mode = Mode::from_bits_truncate(0o400);
/// How a folder under the images folder is opened: to read, and never through a symlink.
const FOLDER_OPEN_FLAGS: OFlag = OFlag::O_RDONLY
    .union(OFlag::O_DIRECTORY)
    .union(OFlag::O_NOFOLLOW)
    .union(OFlag::O_CLOEXEC);

/// Fix the server's local state: with `boxes`, the permissions of the base box images.
///
/// # Errors
///
/// Fails when the image permissions cannot be changed.
pub fn doctor(boxes: bool) -> Result<()> {
    if boxes {
        term_msg_surround("Fixing base box permissions");
        for link in fix_box_permissions(Path::new(SHERPA_IMAGES_PATH), BOX_FILE_MODE)? {
            println!("Skipped symlink {}", link.display());
        }
    }
    Ok(())
}

/// Set the folder `path` and everything under it to the box modes without following a symlink,
/// returning the symlinks it skipped.
#[instrument(level = "debug", fields(path = %path.display()))]
fn fix_box_permissions(path: &Path, file_mode: Mode) -> Result<Vec<PathBuf>> {
    let mut links = Vec::new();
    match Dir::open(path, FOLDER_OPEN_FLAGS, Mode::empty()) {
        Ok(folder) => fix_folder(folder, path, file_mode, &mut links)?,
        Err(Errno::ELOOP) => links.push(path.to_path_buf()),
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to open {}", path.display()));
        }
    }
    Ok(links)
}

/// Set the open `folder` at `path` to `BOX_FOLDER_MODE` and its entries to the box modes,
/// adding each symlink found to `links`.
fn fix_folder(
    mut folder: Dir,
    path: &Path,
    file_mode: Mode,
    links: &mut Vec<PathBuf>,
) -> Result<()> {
    fchmod(&folder, BOX_FOLDER_MODE)
        .with_context(|| format!("Failed to set the mode of {}", path.display()))?;
    let file_mode = if path.file_name() == Some(OsStr::new(SHERPA_BLANK_DISK_DIR)) {
        BLANK_DISK_FILE_MODE
    } else {
        file_mode
    };
    let names = folder
        .iter()
        .map(|entry| entry.map(|entry| entry.file_name().to_owned()))
        .collect::<Result<Vec<CString>, Errno>>()
        .with_context(|| format!("Failed to read {}", path.display()))?;
    for name in names {
        if matches!(name.as_bytes(), b"." | b"..") {
            continue;
        }
        let child = path.join(OsStr::from_bytes(name.as_bytes()));
        match Dir::openat(&folder, name.as_c_str(), FOLDER_OPEN_FLAGS, Mode::empty()) {
            Ok(subfolder) => fix_folder(subfolder, &child, file_mode, links)?,
            Err(Errno::ELOOP) => links.push(child),
            Err(Errno::ENOTDIR) => {
                match fchmodat(
                    &folder,
                    name.as_c_str(),
                    file_mode,
                    FchmodatFlags::NoFollowSymlink,
                ) {
                    Ok(()) => {}
                    Err(Errno::EOPNOTSUPP) => links.push(child),
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!("Failed to set the mode of {}", child.display())
                        });
                    }
                }
            }
            Err(error) => {
                return Err(error).with_context(|| format!("Failed to open {}", child.display()));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, Permissions};
    use std::os::unix::fs::{PermissionsExt, symlink};

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

    #[test]
    fn server_file_modes_020_doctor_boxes_skips_and_names_a_symlink_and_leaves_its_target_as_it_is()
    {
        let root = TempDir::new().unwrap();
        let outside = root.path().join("outside");
        fs::write(&outside, "secret").unwrap();
        fs::set_permissions(&outside, Permissions::from_mode(0o600)).unwrap();
        let images = root.path().join("images");
        let version = images.join("alpine_linux/3.22");
        fs::create_dir_all(&version).unwrap();
        let link = version.join("virtioa.qcow2");
        symlink(&outside, &link).unwrap();
        let image = version.join("virtiob.qcow2");
        fs::write(&image, "image").unwrap();
        fs::set_permissions(&image, Permissions::from_mode(0o644)).unwrap();

        let links = fix_box_permissions(&images, BOX_FILE_MODE).unwrap();

        assert_eq!(links, vec![link]);
        assert_eq!(mode(&outside), 0o600);
        assert_eq!(mode(&image), 0o660);
    }
}
