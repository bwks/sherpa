//! The image folders of the node models renamed in 0.7.0, moved to the new names at startup.
//!
//! A VM image lives at `<images_dir>/<model>/<version>/`, so renaming a model renames its
//! folder. sherpad moves each old folder to its new name, merging into the new folder when two
//! old models became one (`generic_vm` and `generic_unikernel` both into `generic`). A folder
//! or file already at the new path is never replaced: it is left where it is, and logged.
//!
//! A VM overlay disk records its base by path, so an overlay made before the move still names
//! the old folder. While any disk in the storage pool does, the old path is left as a link to
//! the new folder, and a later startup removes the link once none does. A link is state on disk
//! for old labs, not a model name anything accepts. Only folders and links are moved, created
//! or removed: no image file is deleted.

use std::fs;
use std::io::{self, ErrorKind};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::instrument;

use shared::data::REMOVED_MODELS;

/// Move the image folder of each model renamed in 0.7.0 to its new name, and keep or remove
/// the link at the old path as the overlays in `storage_path` need it. Safe to run on every
/// startup: a folder already moved is not moved again.
///
/// When the overlays cannot be read, every link is kept and every moved folder gets one, so no
/// disk loses its base.
///
/// # Errors
///
/// Fails when a folder cannot be read, moved, removed when empty, or linked.
#[instrument]
pub fn move_renamed_model_folders(images_dir: &Path, storage_path: &Path) -> Result<()> {
    let backing_files: Option<Vec<PathBuf>> = match libvirt::overlay_backing_files(storage_path) {
        Ok(found) => Some(found.into_iter().map(|(_, backing)| backing).collect()),
        Err(error) => {
            tracing::warn!(
                error = %format!("{error:#}"),
                "Cannot read the overlay disks; keeping a link at every renamed image folder"
            );
            None
        }
    };
    for (old_name, model, _) in REMOVED_MODELS {
        move_model_folder(
            &images_dir.join(old_name),
            &images_dir.join(model.to_string()),
            backing_files.as_deref(),
        )?;
    }
    Ok(())
}

/// Move the folder at `old` to `new`, or remove the link at `old` once no disk needs it. A disk
/// needs the old path when one of `backing_files` is under it; `None` means the disks are not
/// known, so each one might.
fn move_model_folder(old: &Path, new: &Path, backing_files: Option<&[PathBuf]>) -> Result<()> {
    let needed = backing_files.is_none_or(|files| files.iter().any(|file| file.starts_with(old)));
    let metadata = match fs::symlink_metadata(old) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("Cannot inspect {}", old.display()));
        }
    };
    if metadata.file_type().is_symlink() {
        if !needed {
            fs::remove_file(old)
                .with_context(|| format!("Cannot remove the link {}", old.display()))?;
            tracing::info!(link = %old.display(), "No disk uses the old image folder; link removed");
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        tracing::warn!(path = %old.display(), "Not a folder; left where it is");
        return Ok(());
    }
    merge_folder(old, new)?;
    if old.exists() {
        tracing::warn!(
            folder = %old.display(),
            into = %new.display(),
            "The old image folder still holds entries the new one already has; they are left \
             where they are"
        );
        return Ok(());
    }
    tracing::info!(from = %old.display(), to = %new.display(), "Image folder moved");
    if needed {
        let target = new
            .file_name()
            .with_context(|| format!("{} has no folder name", new.display()))?;
        symlink(target, old).with_context(|| format!("Cannot link {}", old.display()))?;
        tracing::info!(
            link = %old.display(),
            to = %new.display(),
            "Disks still use the old image folder; linked to the new one"
        );
    }
    Ok(())
}

/// Move the folder `source` to `destination`. When `destination` is a folder already, move
/// each entry into it the same way, and remove `source` once it is empty. An entry the
/// destination already has, and that is not a folder in both, stays in `source`.
fn merge_folder(source: &Path, destination: &Path) -> Result<()> {
    let existing = match fs::symlink_metadata(destination) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return fs::rename(source, destination).with_context(|| {
                format!(
                    "Cannot move {} to {}",
                    source.display(),
                    destination.display()
                )
            });
        }
        Err(error) => {
            return Err(error).with_context(|| format!("Cannot inspect {}", destination.display()));
        }
    };
    if !existing.is_dir() {
        tracing::warn!(
            path = %source.display(),
            existing = %destination.display(),
            "Already exists and is not a folder; left where it is"
        );
        return Ok(());
    }
    let entries = fs::read_dir(source)
        .with_context(|| format!("Cannot list {}", source.display()))?
        .collect::<io::Result<Vec<_>>>()
        .with_context(|| format!("Cannot list {}", source.display()))?;
    for entry in entries {
        let path = entry.path();
        let target = destination.join(entry.file_name());
        let is_folder = entry
            .file_type()
            .with_context(|| format!("Cannot inspect {}", path.display()))?
            .is_dir();
        match fs::symlink_metadata(&target) {
            Ok(metadata) if is_folder && metadata.is_dir() => merge_folder(&path, &target)?,
            Ok(_) => tracing::warn!(
                path = %path.display(),
                existing = %target.display(),
                "Already exists; left where it is"
            ),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                fs::rename(&path, &target).with_context(|| {
                    format!("Cannot move {} to {}", path.display(), target.display())
                })?;
            }
            Err(error) => {
                return Err(error).with_context(|| format!("Cannot inspect {}", target.display()));
            }
        }
    }
    let empty = fs::read_dir(source)
        .with_context(|| format!("Cannot list {}", source.display()))?
        .next()
        .is_none();
    if empty {
        fs::remove_dir(source).with_context(|| format!("Cannot remove {}", source.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    /// A QCOW2 header whose backing file is `backing`, as an overlay records its base.
    fn overlay_header(backing: &Path) -> Vec<u8> {
        let name = backing.to_str().unwrap().as_bytes();
        let mut bytes = vec![0; 65536];
        bytes[..4].copy_from_slice(b"QFI\xfb");
        bytes[4..8].copy_from_slice(&3_u32.to_be_bytes());
        bytes[8..16].copy_from_slice(&128_u64.to_be_bytes());
        bytes[16..20].copy_from_slice(&u32::try_from(name.len()).unwrap().to_be_bytes());
        bytes[20..24].copy_from_slice(&16_u32.to_be_bytes());
        bytes[24..32].copy_from_slice(&(2_u64 << 30).to_be_bytes());
        bytes[100..104].copy_from_slice(&104_u32.to_be_bytes());
        bytes[128..128 + name.len()].copy_from_slice(name);
        bytes
    }

    /// An images folder and a storage pool in a temporary directory.
    struct Server {
        _root: tempfile::TempDir,
        images: PathBuf,
        pool: PathBuf,
    }

    impl Server {
        fn new() -> Self {
            let root = tempdir().unwrap();
            let images = root.path().join("images");
            let pool = root.path().join("pool");
            fs::create_dir(&images).unwrap();
            fs::create_dir(&pool).unwrap();
            Self {
                _root: root,
                images,
                pool,
            }
        }

        /// Write `file` under the images folder, with its folders.
        fn image(&self, file: &str) -> PathBuf {
            let path = self.images.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, file).unwrap();
            path
        }

        /// An overlay in the pool on the base `file` under the images folder.
        fn overlay(&self, name: &str, file: &str) {
            fs::write(
                self.pool.join(name),
                overlay_header(&self.images.join(file)),
            )
            .unwrap();
        }

        fn start(&self) {
            move_renamed_model_folders(&self.images, &self.pool).unwrap();
        }

        fn is_link(&self, name: &str) -> bool {
            fs::symlink_metadata(self.images.join(name))
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
        }

        fn read(&self, file: &str) -> String {
            fs::read_to_string(self.images.join(file)).unwrap()
        }
    }

    #[test]
    fn a_folder_no_disk_uses_is_moved_with_no_link() {
        let server = Server::new();
        server.image("cisco_asav/9.20/virtioa.qcow2");

        server.start();

        assert_eq!(
            server.read("cisco_asa/9.20/virtioa.qcow2"),
            "cisco_asav/9.20/virtioa.qcow2"
        );
        assert!(!server.images.join("cisco_asav").exists());
        assert!(!server.is_link("cisco_asav"));
    }

    #[test]
    fn a_folder_an_overlay_uses_is_moved_and_linked_until_no_disk_uses_it() {
        let server = Server::new();
        server.image("arista_veos/4.33/virtioa.qcow2");
        server.image("arista_veos/aboot_iso/aboot.iso");
        server.overlay("leaf1-lab.qcow2", "arista_veos/4.33/virtioa.qcow2");

        server.start();

        assert!(server.is_link("arista_veos"));
        assert_eq!(
            fs::read_link(server.images.join("arista_veos")).unwrap(),
            PathBuf::from("arista_eos")
        );
        // The overlay's base resolves through the link to the moved file.
        assert_eq!(
            server.read("arista_veos/4.33/virtioa.qcow2"),
            "arista_veos/4.33/virtioa.qcow2"
        );
        assert_eq!(
            server.read("arista_eos/aboot_iso/aboot.iso"),
            "arista_veos/aboot_iso/aboot.iso"
        );

        // Running again keeps the link while the overlay is there.
        server.start();
        assert!(server.is_link("arista_veos"));

        // Once the node is redeployed or destroyed, the next start removes the link only.
        fs::remove_file(server.pool.join("leaf1-lab.qcow2")).unwrap();
        server.start();
        assert!(!server.images.join("arista_veos").exists());
        assert!(!server.is_link("arista_veos"));
        assert_eq!(
            server.read("arista_eos/4.33/virtioa.qcow2"),
            "arista_veos/4.33/virtioa.qcow2"
        );
    }

    #[test]
    fn an_overlay_on_another_model_does_not_keep_a_link() {
        let server = Server::new();
        server.image("juniper_vsrxv3/23.4/virtioa.qcow2");
        server.image("ubuntu_linux/24.04/virtioa.qcow2");
        server.overlay("dev1-lab.qcow2", "ubuntu_linux/24.04/virtioa.qcow2");

        server.start();

        assert!(!server.images.join("juniper_vsrxv3").exists());
        assert!(
            server
                .images
                .join("juniper_srx/23.4/virtioa.qcow2")
                .exists()
        );
    }

    #[test]
    fn two_old_folders_of_one_model_merge_into_its_folder() {
        let server = Server::new();
        server.image("generic_vm/1.0/virtioa.qcow2");
        server.image("generic_unikernel/1.0/kernel.elf");
        server.image("generic_unikernel/2.0/kernel.elf");
        // The container models had empty folders.
        fs::create_dir(server.images.join("generic_container")).unwrap();
        fs::create_dir(server.images.join("arista_ceos")).unwrap();
        server.image("arista_veos/4.33/virtioa.qcow2");

        server.start();

        for file in [
            "generic/1.0/virtioa.qcow2",
            "generic/1.0/kernel.elf",
            "generic/2.0/kernel.elf",
            "arista_eos/4.33/virtioa.qcow2",
        ] {
            assert!(server.images.join(file).exists(), "{file}");
        }
        for old in [
            "generic_vm",
            "generic_unikernel",
            "generic_container",
            "arista_ceos",
            "arista_veos",
        ] {
            assert!(!server.images.join(old).exists(), "{old}");
        }
    }

    #[test]
    fn a_file_the_new_folder_already_has_is_left_in_the_old_one() {
        let server = Server::new();
        server.image("cisco_ftdv/7.4/virtioa.qcow2");
        server.image("cisco_ftdv/7.6/virtioa.qcow2");
        fs::create_dir_all(server.images.join("cisco_ftd/7.4")).unwrap();
        fs::write(server.images.join("cisco_ftd/7.4/virtioa.qcow2"), "new").unwrap();

        server.start();

        assert_eq!(server.read("cisco_ftd/7.4/virtioa.qcow2"), "new");
        assert_eq!(
            server.read("cisco_ftdv/7.4/virtioa.qcow2"),
            "cisco_ftdv/7.4/virtioa.qcow2"
        );
        assert_eq!(
            server.read("cisco_ftd/7.6/virtioa.qcow2"),
            "cisco_ftdv/7.6/virtioa.qcow2"
        );
        assert!(!server.is_link("cisco_ftdv"));
    }

    #[test]
    fn unreadable_disks_keep_a_link_at_every_moved_folder() {
        let server = Server::new();
        server.image("juniper_vrouter/23.2/virtioa.qcow2");
        // A damaged QCOW2 disk: its dependencies cannot be known.
        fs::write(server.pool.join("broken.qcow2"), b"QFI\xfb").unwrap();

        server.start();

        assert!(server.is_link("juniper_vrouter"));
        assert!(
            server
                .images
                .join("juniper_vrouter/23.2/virtioa.qcow2")
                .exists()
        );
    }

    #[test]
    fn nothing_to_move_changes_nothing() {
        let server = Server::new();
        server.image("ubuntu_linux/24.04/virtioa.qcow2");

        server.start();
        server.start();

        assert_eq!(fs::read_dir(&server.images).unwrap().count(), 1);
    }
}
