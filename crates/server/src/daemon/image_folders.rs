//! The image folders of the retired node model names, moved to their replacements at startup.
//!
//! A VM image lives at `<images_dir>/<model>/<version>/`, so renaming a model renames its
//! folder. sherpad moves each old folder to its new name, merging into the new folder when two
//! old models became one (`generic_vm` and `generic_unikernel` both into `generic`). A folder
//! or file already at the new path is never replaced: it is left where it is, and logged.
//!
//! A VM overlay disk records its base by path, so an overlay made before the move still names
//! the old folder. While any disk in the storage pool does, the old path is left as a link to
//! the new folder, and a later startup removes the link once none does. A link is state on disk
//! for old labs, not a model name anything accepts. libvirt reports each disk's base: sherpad
//! cannot read the disks libvirt has handed to QEMU. Only folders and links are moved, created
//! or removed: no image file is deleted.

use std::fs;
use std::io::{self, ErrorKind};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::instrument;

use shared::data::REMOVED_MODELS;

/// Move the image folder of each retired model name to its replacement, and keep or remove
/// the link at the old path as the disks need it. `backing_files` is the base each disk in the
/// storage pool records (`libvirt::pool_backed_volumes`); `None` when they cannot be read, and
/// then every link is kept and every moved folder gets one, so no disk loses its base. Safe to
/// run on every startup: a folder already moved is not moved again.
///
/// # Errors
///
/// Fails when a folder cannot be read, moved, removed when empty, or linked.
#[instrument(skip(backing_files))]
pub fn move_renamed_model_folders(
    images_dir: &Path,
    backing_files: Option<&[PathBuf]>,
) -> Result<()> {
    if backing_files.is_none() {
        tracing::warn!(
            "The storage pool's disks are not known; keeping a link at every renamed image folder"
        );
    }
    for (old_name, model, _) in REMOVED_MODELS {
        move_model_folder(
            &images_dir.join(old_name),
            &images_dir.join(model.to_string()),
            backing_files,
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

    /// An images folder in a temporary directory, and the bases the storage pool's disks
    /// record.
    struct Server {
        _root: tempfile::TempDir,
        images: PathBuf,
        backing_files: Option<Vec<PathBuf>>,
    }

    impl Server {
        fn new() -> Self {
            let root = tempdir().unwrap();
            let images = root.path().join("images");
            fs::create_dir(&images).unwrap();
            Self {
                _root: root,
                images,
                backing_files: Some(Vec::new()),
            }
        }

        /// Write `file` under the images folder, with its folders.
        fn image(&self, file: &str) -> PathBuf {
            let path = self.images.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, file).unwrap();
            path
        }

        /// A disk in the pool on the base `file` under the images folder.
        fn overlay(&mut self, file: &str) {
            let base = self.images.join(file);
            self.backing_files.get_or_insert_default().push(base);
        }

        fn start(&self) {
            move_renamed_model_folders(&self.images, self.backing_files.as_deref()).unwrap();
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
    fn image_folder_rename_001_a_folder_no_disk_uses_is_moved_with_no_link() {
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
    fn image_folder_rename_002_a_folder_an_overlay_uses_is_moved_and_linked_until_no_disk_uses_it()
    {
        let mut server = Server::new();
        server.image("arista_veos/4.33/virtioa.qcow2");
        server.image("arista_veos/aboot_iso/aboot.iso");
        server.overlay("arista_veos/4.33/virtioa.qcow2");

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
        server.backing_files = Some(Vec::new());
        server.start();
        assert!(!server.images.join("arista_veos").exists());
        assert!(!server.is_link("arista_veos"));
        assert_eq!(
            server.read("arista_eos/4.33/virtioa.qcow2"),
            "arista_veos/4.33/virtioa.qcow2"
        );
    }

    #[test]
    fn image_folder_rename_003_an_overlay_on_another_model_does_not_keep_a_link() {
        let mut server = Server::new();
        server.image("juniper_vsrxv3/23.4/virtioa.qcow2");
        server.image("ubuntu_linux/24.04/virtioa.qcow2");
        server.overlay("ubuntu_linux/24.04/virtioa.qcow2");

        server.start();

        assert!(!server.images.join("juniper_vsrxv3").exists());
        assert!(
            server
                .images
                .join("juniper_srxv3/23.4/virtioa.qcow2")
                .exists()
        );
    }

    #[test]
    fn image_folder_rename_004_ios_folders_move_beside_the_iol_folders_and_only_the_used_one_is_linked()
     {
        let mut server = Server::new();
        server.image("cisco_iosv/159-3.m10/virtioa.qcow2");
        server.image("cisco_iosvl2/20200929/virtioa.qcow2");
        server.image("cisco_iol/17.16/iol.bin");
        server.image("cisco_ioll2/17.16/iol.bin");
        server.overlay("cisco_iosv/159-3.m10/virtioa.qcow2");

        server.start();

        assert_eq!(
            server.read("cisco_ios/159-3.m10/virtioa.qcow2"),
            "cisco_iosv/159-3.m10/virtioa.qcow2"
        );
        assert_eq!(
            server.read("cisco_iosl2/20200929/virtioa.qcow2"),
            "cisco_iosvl2/20200929/virtioa.qcow2"
        );
        assert!(server.is_link("cisco_iosv"));
        assert!(!server.images.join("cisco_iosvl2").exists());
        assert!(!server.is_link("cisco_iosvl2"));
        assert_eq!(
            server.read("cisco_iol/17.16/iol.bin"),
            "cisco_iol/17.16/iol.bin"
        );
        assert_eq!(
            server.read("cisco_ioll2/17.16/iol.bin"),
            "cisco_ioll2/17.16/iol.bin"
        );
    }

    #[test]
    fn image_folder_rename_005_two_old_folders_of_one_model_merge_into_its_folder() {
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
    fn image_folder_rename_006_a_file_the_new_folder_already_has_is_left_in_the_old_one() {
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
    fn image_folder_rename_007_unknown_disks_keep_a_link_at_every_moved_folder() {
        let mut server = Server::new();
        server.image("juniper_vrouter/23.2/virtioa.qcow2");
        // The pool's disks cannot be listed.
        server.backing_files = None;

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
    fn image_folder_rename_008_nothing_to_move_changes_nothing() {
        let server = Server::new();
        server.image("ubuntu_linux/24.04/virtioa.qcow2");

        server.start();
        server.start();

        assert_eq!(fs::read_dir(&server.images).unwrap().count(), 1);
    }
}
