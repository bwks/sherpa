//! The Docker store of a container image version for sandboxes: a disk that holds Docker's data
//! with the image loaded, made once per version by a Flatcar VM and moved to the images folder,
//! which each sandbox of the version boots an overlay of.

use std::fs::Permissions;
use std::io::ErrorKind;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use askama::Template;
use futures_util::StreamExt;
use libvirt::VolumeOwner;
use nix::unistd::{getegid, geteuid};
use sha2::{Digest, Sha256};
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;
use tokio::{task, time};
use tracing::instrument;

use shared::data::{NodeModel, StatusKind};
use template::{FlatcarDomain, store_vm_ignition_json};

use super::create::{DomainDisk, SandboxHost, pool_path, start_domain};
use super::guest::write_config_drive;
use super::status::{Status, StatusWatch};
use crate::daemon::storage::restrict_dir;
use crate::konst::{
    CONSOLE_GROUP, CONSOLE_LAB_DIR_MODE, IMAGE_FILE_MODE, IMAGE_FOLDER_MODE,
    SANDBOX_STORE_FILENAME, SHERPA_SANDBOX_RUN_DIR,
};
use crate::services::image_store::ImagePaths;

/// The size of a Docker store: the image, and what each node of it writes.
const STORE_DISK_GB: u16 = 32;
/// The memory of the VM that makes a store.
const STORE_VM_MEMORY_MIB: u16 = 2048;
/// The vCPUs of the VM that makes a store.
const STORE_VM_VCPUS: u8 = 2;
/// How long the VM that makes a store has to boot and load the image.
const STORE_VM_LOAD: Duration = Duration::from_mins(30);
/// How long the VM that made a store has to power off.
const STORE_VM_SHUTDOWN: Duration = Duration::from_secs(120);
/// The block size of a disk; the image archive is padded to a whole number of them.
const SECTOR_BYTES: u64 = 512;
/// The mode of the image archive, which libvirt hands to QEMU.
const ARCHIVE_MODE: u32 = 0o644;
/// The start of the name of a store build's VM and of the pool volumes it makes, whose `.` no
/// node's domain or volume has.
const BUILD_PREFIX: &str = "sherpa-store.";
/// The end of the volume name of a store VM's disk.
const VM_DISK_SUFFIX: &str = "-hdd.qcow2";
/// The end of the volume name of a store VM's config drive.
const VM_CONFIG_DRIVE_SUFFIX: &str = ".iso";
/// The end of an image archive's volume name.
const ARCHIVE_SUFFIX: &str = ".tar";
/// The end of the volume name of the disk a store is made in, before it is moved to the images
/// folder.
const PARTIAL_SUFFIX: &str = ".qcow2.part";
/// The start of a store VM's status socket, which is named by a hash of the store: a version is
/// too long for a socket path.
const STATUS_PREFIX: &str = "store-";
/// The end of a store VM's status socket.
const STATUS_SUFFIX: &str = ".status";
/// The hex digits of the hash a status socket is named by.
const STATUS_HASH_LEN: usize = 16;
/// The end of the file a store is copied to in its folder when it cannot be renamed there.
const COPY_SUFFIX: &str = ".part";

/// The names of the Docker store of one container image version and of what makes it.
pub(crate) struct Store {
    model: NodeModel,
    version: String,
}

impl Store {
    /// The store of version `version` of model `model`'s container image.
    pub(crate) fn of(model: NodeModel, version: &str) -> Self {
        Self {
            model,
            version: version.to_owned(),
        }
    }

    /// The store's file in the images folder of `paths`.
    pub(crate) fn path(&self, paths: &ImagePaths) -> PathBuf {
        paths
            .sandbox_store_dir(self.model, &self.version)
            .join(SANDBOX_STORE_FILENAME)
    }

    /// The domain name of the VM that makes the store, which the build's volumes start with.
    fn vm(&self) -> String {
        format!("{BUILD_PREFIX}{}.{}", self.model, self.version)
    }

    /// The volume the store is made in, moved to the images folder once it is complete.
    fn partial(&self) -> String {
        format!("{}{PARTIAL_SUFFIX}", self.vm())
    }

    /// The volume of the image archive the VM loads.
    fn archive(&self) -> String {
        format!("{}{ARCHIVE_SUFFIX}", self.vm())
    }

    /// The VM's disk, an overlay on the base image.
    fn vm_disk(&self) -> String {
        format!("{}{VM_DISK_SUFFIX}", self.vm())
    }

    /// The VM's config drive.
    fn vm_config_drive(&self) -> String {
        format!("{}{VM_CONFIG_DRIVE_SUFFIX}", self.vm())
    }

    /// The folder in run folder `run_dir` the VM's config drive is staged in.
    fn vm_stage(&self, run_dir: &Path) -> PathBuf {
        run_dir.join(format!(".{}", self.vm()))
    }

    /// The VM's status socket.
    fn status_socket(&self) -> String {
        let hash = hex::encode(Sha256::digest(self.vm()));
        format!(
            "{SHERPA_SANDBOX_RUN_DIR}/{STATUS_PREFIX}{}{STATUS_SUFFIX}",
            &hash[..STATUS_HASH_LEN]
        )
    }
}

/// Whether pool volume `volume` is one a store build makes and removes again: the VM's disk or
/// config drive, the image archive or the partial store.
pub(super) fn is_build_volume(volume: &str) -> bool {
    volume.starts_with(BUILD_PREFIX)
        && [
            VM_DISK_SUFFIX,
            VM_CONFIG_DRIVE_SUFFIX,
            ARCHIVE_SUFFIX,
            PARTIAL_SUFFIX,
        ]
        .iter()
        .any(|suffix| volume.ends_with(suffix))
}

/// Whether domain `name` is the VM of a store build.
pub(super) fn is_build_domain(name: &str) -> bool {
    name.starts_with(BUILD_PREFIX)
}

/// Whether `name`, in the sandbox run folder, is the staging folder or status socket of a store
/// build.
pub(super) fn is_build_file(name: &str) -> bool {
    name.starts_with(&format!(".{BUILD_PREFIX}"))
        || (name.starts_with(STATUS_PREFIX) && name.ends_with(STATUS_SUFFIX))
}

/// The path of the Docker store of image `image`, version `version` of model `model`, made
/// first, by a VM on base image `base`, when there is none. The caller holds the image's lease,
/// so it is neither deleted nor imported again while the store is made.
///
/// # Errors
///
/// Fails when the server's Docker does not have the image, or the store cannot be made.
#[instrument(skip(host), level = "debug")]
pub(super) async fn ensure_docker_store(
    host: &SandboxHost,
    base: &str,
    model: NodeModel,
    version: &str,
    image: &str,
) -> Result<String> {
    let store = Store::of(model, version);
    let path = store.path(&ImagePaths::installed());
    let path = path
        .to_str()
        .with_context(|| format!("The store path {} is not UTF-8", path.display()))?
        .to_owned();
    host.disk_locks
        .build(
            &path,
            || {
                let _ = host.progress.send_status(
                    format!("Waiting for the sandbox Docker store of image {image} to be made"),
                    StatusKind::Waiting,
                );
            },
            build_store(
                host.clone(),
                base.to_owned(),
                image.to_owned(),
                store,
                path.clone(),
            ),
        )
        .await
}

/// The path `path` of `store` of image `image`, made by a VM on base image `base` when there is
/// none, run while its lock is held.
async fn build_store(
    host: SandboxHost,
    base: String,
    image: String,
    store: Store,
    path: String,
) -> Result<String> {
    if fs::metadata(&path)
        .await
        .is_ok_and(|metadata| metadata.is_file())
    {
        return Ok(path);
    }
    tracing::info!(%image, store = %path, "Making the sandbox Docker store of the image");
    let _ = host.progress.send_status(
        format!(
            "Making the sandbox Docker store of image {image}: a VM loads the image, which takes \
             minutes, once per image"
        ),
        StatusKind::Progress,
    );
    let made = make_store(&host, &base, &image, &store).await;
    if let Err(removal) = remove_store_vm(&host, &store, made.is_err()).await {
        tracing::warn!(store = %path, error = %removal, "Cannot remove the VM that made a Docker store");
    }
    made?;
    let moved = place_store(Path::new(&pool_path(&store.partial())), Path::new(&path)).await;
    if let Err(removal) = remove_store_vm(&host, &store, true).await {
        tracing::warn!(store = %path, error = %removal, "Cannot remove the partial Docker store");
    }
    moved?;
    let _ = host.progress.send_status(
        format!("Made the sandbox Docker store of image {image}"),
        StatusKind::Done,
    );
    Ok(path)
}

/// Make `store` of image `image`: write the image's archive, boot a Flatcar VM on base image
/// `base` that loads it into an empty disk owned by sherpad and powers off, and wait until it
/// has.
async fn make_store(host: &SandboxHost, base: &str, image: &str, store: &Store) -> Result<()> {
    remove_store_vm(host, store, true).await?;
    let archive = pool_path(&store.archive());
    write_image_archive(host, image, &archive).await?;
    let stage = store.vm_stage(Path::new(SHERPA_SANDBOX_RUN_DIR));
    let drive = store.vm_config_drive();
    let ignition = store_vm_ignition_json()?;
    let config_drive = task::spawn_blocking(move || {
        restrict_dir(
            Path::new(SHERPA_SANDBOX_RUN_DIR),
            CONSOLE_LAB_DIR_MODE,
            Some(CONSOLE_GROUP),
        )?;
        write_config_drive(&stage, &drive, &ignition, None, &[])
    })
    .await
    .context("The Docker store config drive write panicked")??;
    let status_socket = store.status_socket();
    let vm = store.vm();
    let disk = pool_path(&store.vm_disk());
    let docker_disk = pool_path(&store.partial());
    let xml = FlatcarDomain {
        name: vm.clone(),
        memory: STORE_VM_MEMORY_MIB,
        vcpus: STORE_VM_VCPUS,
        qemu_bin: host.qemu_bin.clone(),
        disk: disk.clone(),
        config_drive,
    }
    .store_vm(docker_disk.clone(), archive, status_socket.clone())
    .render()
    .with_context(|| format!("Cannot render the domain of VM {vm}"))?;
    let disks = vec![
        DomainDisk::Overlay {
            base: base.to_owned(),
            path: disk,
        },
        DomainDisk::Blank {
            path: docker_disk,
            size_gb: STORE_DISK_GB,
            owner: VolumeOwner {
                uid: geteuid().as_raw(),
                gid: getegid().as_raw(),
            },
        },
    ];
    start_domain(&host.qemu, &vm, xml, disks).await?;
    let mut watch = StatusWatch::connect(&status_socket, &vm).await?;
    time::timeout(STORE_VM_LOAD, watch.next())
        .await
        .with_context(|| {
            format!(
                "VM {vm} did not load image {image} within {} seconds",
                STORE_VM_LOAD.as_secs()
            )
        })??
        .expect(&Status::Loaded, &vm)?;
    let qemu = Arc::clone(&host.qemu);
    task::spawn_blocking(move || {
        let conn = qemu.connect()?;
        libvirt::wait_until_stopped(&conn, &vm, STORE_VM_SHUTDOWN)
    })
    .await
    .context("The Docker store VM wait panicked")?
}

/// Move the finished store `partial` to `destination` in the images folder, making its folders
/// with the images folder's mode: renamed when sherpad owns it and it is on the same filesystem,
/// and otherwise copied next to `destination` and renamed onto it, so the store is never seen
/// half written. The store has an imported image's mode. A copy leaves `partial` for the caller
/// to remove.
///
/// # Errors
///
/// Fails when a folder cannot be made, or the store cannot be renamed or copied.
async fn place_store(partial: &Path, destination: &Path) -> Result<()> {
    let folder = destination
        .parent()
        .with_context(|| format!("{} has no folder", destination.display()))?;
    make_store_folders(folder).await?;
    let owned = fs::metadata(partial)
        .await
        .with_context(|| format!("Cannot read {}", partial.display()))?
        .uid()
        == geteuid().as_raw();
    if owned {
        fs::set_permissions(partial, Permissions::from_mode(IMAGE_FILE_MODE))
            .await
            .with_context(|| format!("Cannot set the mode of {}", partial.display()))?;
        match fs::rename(partial, destination).await {
            Ok(()) => {
                tracing::info!(store = %destination.display(), "Moved the Docker store to its folder");
                return Ok(());
            }
            Err(error) if error.kind() == ErrorKind::CrossesDevices => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Cannot move {} to {}",
                        partial.display(),
                        destination.display()
                    )
                });
            }
        }
    }
    copy_store(partial, destination).await?;
    tracing::info!(store = %destination.display(), owned, "Copied the Docker store to its folder");
    Ok(())
}

/// Make `folder` and the missing folders above it, giving `folder` and its parent, the model's
/// store folder, the images folder's mode.
async fn make_store_folders(folder: &Path) -> Result<()> {
    fs::create_dir_all(folder)
        .await
        .with_context(|| format!("Cannot create {}", folder.display()))?;
    for dir in [Some(folder), folder.parent()].into_iter().flatten() {
        fs::set_permissions(dir, Permissions::from_mode(IMAGE_FOLDER_MODE))
            .await
            .with_context(|| format!("Cannot set the mode of {}", dir.display()))?;
    }
    Ok(())
}

/// Copy the store `partial` to a file next to `destination`, give it an imported image's mode,
/// write it to disk and rename it onto `destination`, removing the copy when a step fails.
async fn copy_store(partial: &Path, destination: &Path) -> Result<()> {
    let mut copy = destination.as_os_str().to_owned();
    copy.push(COPY_SUFFIX);
    let copy = PathBuf::from(copy);
    let copied = write_copy(partial, &copy, destination).await;
    if copied.is_err() {
        let _ = fs::remove_file(&copy).await;
    }
    copied
}

/// Copy `partial` to `copy` with an imported image's mode, write it to disk, and rename it onto
/// `destination`.
async fn write_copy(partial: &Path, copy: &Path, destination: &Path) -> Result<()> {
    fs::copy(partial, copy)
        .await
        .with_context(|| format!("Cannot copy {} to {}", partial.display(), copy.display()))?;
    fs::set_permissions(copy, Permissions::from_mode(IMAGE_FILE_MODE))
        .await
        .with_context(|| format!("Cannot set the mode of {}", copy.display()))?;
    File::open(copy)
        .await
        .with_context(|| format!("Cannot open {}", copy.display()))?
        .sync_all()
        .await
        .with_context(|| format!("Cannot write {}", copy.display()))?;
    fs::rename(copy, destination).await.with_context(|| {
        format!(
            "Cannot move {} to {}",
            copy.display(),
            destination.display()
        )
    })
}

/// Write the archive of image `image` from the server's Docker to `path`, padded with zeros to
/// whole sectors so QEMU serves all of it as a disk.
///
/// # Errors
///
/// Fails when Docker cannot export the image or the archive cannot be written.
async fn write_image_archive(host: &SandboxHost, image: &str, path: &str) -> Result<()> {
    let mut file = File::create(path)
        .await
        .with_context(|| format!("Cannot create {path}"))?;
    let mut chunks = host.docker.export_image(image);
    let mut written: u64 = 0;
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.with_context(|| format!("Cannot export image {image}"))?;
        file.write_all(&chunk)
            .await
            .with_context(|| format!("Cannot write {path}"))?;
        written += chunk.len() as u64;
    }
    let padding = written.next_multiple_of(SECTOR_BYTES) - written;
    file.set_len(written + padding)
        .await
        .with_context(|| format!("Cannot pad {path}"))?;
    file.sync_all()
        .await
        .with_context(|| format!("Cannot write {path}"))?;
    fs::set_permissions(path, Permissions::from_mode(ARCHIVE_MODE))
        .await
        .with_context(|| format!("Cannot set the mode of {path}"))
}

/// Remove the VM that makes `store`, its disk, config drive and image archive, and with
/// `delete_partial` the store's partial volume, where they are.
async fn remove_store_vm(host: &SandboxHost, store: &Store, delete_partial: bool) -> Result<()> {
    let qemu = Arc::clone(&host.qemu);
    let vm = store.vm();
    let mut volumes = vec![store.vm_disk(), store.vm_config_drive(), store.archive()];
    if delete_partial {
        volumes.push(store.partial());
    }
    task::spawn_blocking(move || {
        let conn = qemu.connect()?;
        libvirt::remove_domain(&conn, &vm)?;
        let present = libvirt::list_pool_volumes(&conn)?;
        for volume in volumes.iter().filter(|volume| present.contains(volume)) {
            libvirt::delete_disk(&conn, volume)?;
        }
        Ok(())
    })
    .await
    .context("The Docker store VM removal panicked")?
}

#[cfg(test)]
mod tests {
    use std::fs as std_fs;

    use libvirt::Qemu;
    use tempfile::TempDir;
    use virt::domain::Domain;

    use super::*;
    use crate::konst::UNIX_SOCKET_PATH_MAX;
    use crate::services::sandbox::remove_unfinished_builds;

    /// The mode bits of `path`.
    fn mode(path: &Path) -> u32 {
        std_fs::metadata(path).unwrap().permissions().mode() & 0o7777
    }

    #[test]
    fn sandbox_105_a_store_is_the_versions_disk_in_the_models_sandbox_folder_of_the_images_folder()
    {
        let paths = ImagePaths {
            images: PathBuf::from("/opt/sherpa/images"),
        };
        let store = Store::of(NodeModel::AristaEos, "4.35.0F");
        assert_eq!(
            store.path(&paths),
            Path::new("/opt/sherpa/images/arista_eos/sandbox/4.35.0F/virtiob.qcow2")
        );
        assert_eq!(store.vm(), "sherpa-store.arista_eos.4.35.0F");
    }

    #[test]
    fn sandbox_065_the_files_a_store_build_makes_are_its_own_and_a_nodes_are_not() {
        let store = Store::of(NodeModel::NokiaSrlinux, "26.7.2");
        for volume in [
            store.vm_disk(),
            store.vm_config_drive(),
            store.archive(),
            store.partial(),
        ] {
            assert!(is_build_volume(&volume), "{volume}");
        }
        for volume in [
            "ceos1-abc12345-docker.qcow2",
            "sherpa-store-abc12345-hdd.qcow2",
            "sherpa-store.nokia_srlinux.26.7.2.qcow2",
            "sherpa-images.qcow2",
        ] {
            assert!(!is_build_volume(volume), "{volume}");
        }
        assert!(is_build_domain(&store.vm()));
        assert!(!is_build_domain("sherpa-store-abc12345"));
        let longest = Store::of(NodeModel::JuniperEvolved, &"1".repeat(64)).status_socket();
        assert!(longest.len() <= UNIX_SOCKET_PATH_MAX, "{longest}");
        let name = Path::new(&longest).file_name().unwrap().to_str().unwrap();
        assert!(is_build_file(name), "{name}");
        assert_ne!(
            store.status_socket(),
            Store::of(NodeModel::NokiaSrlinux, "26.7.3").status_socket()
        );
    }

    #[tokio::test]
    async fn sandbox_106_a_finished_store_is_moved_to_its_version_folder_with_an_imported_images_modes()
     {
        let root = TempDir::new().unwrap();
        let partial = root.path().join("sherpa-store.arista_eos.1.qcow2.part");
        std_fs::write(&partial, b"store").unwrap();
        std_fs::set_permissions(&partial, Permissions::from_mode(0o600)).unwrap();
        let destination = root.path().join("arista_eos/sandbox/1/virtiob.qcow2");

        place_store(&partial, &destination).await.unwrap();

        assert_eq!(std_fs::read(&destination).unwrap(), b"store");
        assert!(!partial.exists());
        assert_eq!(mode(&destination), IMAGE_FILE_MODE);
        for folder in ["arista_eos/sandbox/1", "arista_eos/sandbox"] {
            assert_eq!(
                mode(&root.path().join(folder)),
                IMAGE_FOLDER_MODE,
                "{folder}"
            );
        }
    }

    #[tokio::test]
    async fn sandbox_107_a_store_that_cannot_be_renamed_is_copied_next_to_its_path_and_renamed_onto_it()
     {
        let root = TempDir::new().unwrap();
        let partial = root.path().join("sherpa-store.arista_eos.1.qcow2.part");
        std_fs::write(&partial, b"store").unwrap();
        let folder = root.path().join("arista_eos/sandbox/1");
        std_fs::create_dir_all(&folder).unwrap();
        let destination = folder.join(SANDBOX_STORE_FILENAME);

        copy_store(&partial, &destination).await.unwrap();

        assert_eq!(std_fs::read(&destination).unwrap(), b"store");
        assert_eq!(mode(&destination), IMAGE_FILE_MODE);
        assert_eq!(
            std_fs::read_dir(&folder).unwrap().count(),
            1,
            "the copy was left beside the store"
        );
        assert!(partial.is_file(), "the caller removes the pool volume");
        assert!(
            copy_store(&root.path().join("missing"), &destination)
                .await
                .is_err()
        );
        assert_eq!(std_fs::read_dir(&folder).unwrap().count(), 1);
    }

    #[tokio::test]
    #[ignore = "Requires libvirt and sherpa-pool, and no store being made; run as root"]
    async fn sandbox_066_sherpad_removes_what_an_unfinished_store_build_left() {
        let qemu = Arc::new(Qemu::default());
        let conn = qemu.connect().unwrap();
        let store = Store::of(NodeModel::NokiaSrlinux, "st066");
        let owner = VolumeOwner {
            uid: geteuid().as_raw(),
            gid: getegid().as_raw(),
        };
        for volume in [store.vm_disk(), store.archive(), store.partial()] {
            libvirt::create_blank_qcow2(&conn, &pool_path(&volume), 1, owner).unwrap();
        }
        Domain::define_xml(
            &conn,
            &format!(
                "<domain type='kvm'><name>{}</name><memory unit='MiB'>64</memory>\
                 <os><type arch='x86_64'>hvm</type></os></domain>",
                store.vm()
            ),
        )
        .unwrap();
        let run = TempDir::new().unwrap();
        let stage = store.vm_stage(run.path());
        std_fs::create_dir_all(stage.join("openstack")).unwrap();
        let socket = run
            .path()
            .join(Path::new(&store.status_socket()).file_name().unwrap());
        std_fs::write(&socket, "").unwrap();

        let swept = remove_unfinished_builds(&qemu, run.path()).await;

        let volumes = libvirt::list_pool_volumes(&conn).unwrap();
        let domains = libvirt::list_domain_names(&conn).unwrap();
        swept.unwrap();
        for volume in [store.vm_disk(), store.archive(), store.partial()] {
            assert!(!volumes.contains(&volume), "{volume}");
        }
        assert!(!domains.contains(&store.vm()));
        assert!(!stage.exists());
        assert!(!socket.exists());
    }
}
