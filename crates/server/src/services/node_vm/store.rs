//! The Docker store of an image for node VMs: a disk that holds Docker's data with the image
//! loaded, which each node VM of the image boots an overlay of. A Flatcar VM of its own makes
//! it once per image, loading the image's archive from a disk of its own.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::{task, time};
use tracing::instrument;

use shared::konst::SHERPA_STORAGE_POOL_PATH;
use template::{NodeVmDomainTemplate, store_vm_ignition_json};

use super::base::{NODE_VM_IMAGE_DIR, base_image, base_kernel, kernel_cmdline, kernel_usr_hash};
use super::create::{NodeVmHost, start_domain};
use super::guest::write_config_drive;
use super::status::{self, Status};
use crate::services::console;

/// The size of a Docker store: the image, and what each node of it writes.
const STORE_DISK_GB: u16 = 32;
/// The hex digits of the image ID a store is named by.
const STORE_ID_LEN: usize = 16;
/// The memory of the VM that makes a store.
const STORE_VM_MEMORY_MIB: u64 = 2048;
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

/// The names of the Docker store of one image ID and of what makes it.
struct Store {
    id: String,
}

impl Store {
    /// The store of image ID `image_id` (`sha256:HEX`), or `None` when the ID is not one.
    fn of(image_id: &str) -> Option<Self> {
        let id = image_id.strip_prefix("sha256:")?.get(..STORE_ID_LEN)?;
        id.bytes()
            .all(|byte| byte.is_ascii_hexdigit())
            .then(|| Self { id: id.to_owned() })
    }

    /// The store's volume.
    fn volume(&self) -> String {
        format!("flatcar-docker-{}.qcow2", self.id)
    }

    /// The volume the store is made in, renamed to the store once it is complete.
    fn partial(&self) -> String {
        format!("flatcar-docker-{}.qcow2.part", self.id)
    }

    /// The volume of the image archive the VM loads.
    fn archive(&self) -> String {
        format!("flatcar-docker-{}.tar", self.id)
    }

    /// The domain name of the VM that makes the store.
    fn vm(&self) -> String {
        format!("sherpa-flatcar-docker-{}", self.id)
    }

    /// The VM's disk, an overlay on the base image.
    fn vm_disk(&self) -> String {
        format!("{}-hdd.qcow2", self.vm())
    }

    /// The VM's config drive.
    fn vm_config_drive(&self) -> String {
        format!("{}.iso", self.vm())
    }

    /// The VM's status socket in lab `lab_id`'s console folder.
    fn status_socket(&self, lab_id: &str) -> Result<String> {
        console::lab_socket(lab_id, &format!("store-{}.status", self.id))
    }
}

/// The storage pool path of volume `volume`.
fn pool_path(volume: &str) -> String {
    format!("{SHERPA_STORAGE_POOL_PATH}/{volume}")
}

/// The path of the Docker store of image `image`, made first when there is none, for a node of
/// lab `lab_id`.
///
/// # Errors
///
/// Fails when the server's Docker does not have the image, or the store cannot be made.
#[instrument(skip(host), level = "debug")]
pub(super) async fn ensure_docker_store(
    host: &NodeVmHost,
    lab_id: &str,
    image: &str,
) -> Result<String> {
    let id = host
        .docker
        .inspect_image(image)
        .await
        .with_context(|| format!("Cannot inspect image {image}"))?
        .id
        .with_context(|| format!("Image {image} has no ID"))?;
    let store = Store::of(&id).with_context(|| format!("Image {image} has ID '{id}'"))?;
    let path = pool_path(&store.volume());
    let lock = host.store_locks.of(&store.volume());
    let _guard = lock.lock().await;
    if Path::new(&path).is_file() {
        return Ok(path);
    }
    tracing::info!(%image, store = %store.volume(), "Making the node VM Docker store of the image");
    let made = make_store(host, lab_id, image, &store).await;
    if let Err(removal) = remove_store_vm(host, &store, made.is_err()).await {
        tracing::warn!(store = %store.volume(), error = %removal, "Cannot remove the VM that made a Docker store");
    }
    made?;
    let partial = pool_path(&store.partial());
    fs::rename(&partial, &path).with_context(|| format!("Cannot rename {partial} to {path}"))?;
    Ok(path)
}

/// Make `store` of image `image`: write the image's archive, boot a Flatcar VM that loads it
/// into an empty disk and powers off, and wait until it has.
async fn make_store(host: &NodeVmHost, lab_id: &str, image: &str, store: &Store) -> Result<()> {
    remove_store_vm(host, store, true).await?;
    let archive = pool_path(&store.archive());
    write_image_archive(host, image, &archive).await?;
    let config_drive = write_config_drive(
        &Path::new(&host.images_dir)
            .join(NODE_VM_IMAGE_DIR)
            .join(format!(".{}", store.vm())),
        &store.vm_config_drive(),
        &store_vm_ignition_json()?,
        &[],
    )?;
    let status_socket = store.status_socket(lab_id)?;
    let domain = NodeVmDomainTemplate {
        name: store.vm(),
        memory_mib: STORE_VM_MEMORY_MIB,
        vcpus: STORE_VM_VCPUS,
        kernel: base_kernel(&host.images_dir),
        cmdline: kernel_cmdline(&kernel_usr_hash(&host.images_dir)?, true),
        disk: pool_path(&store.vm_disk()),
        docker_disk: pool_path(&store.partial()),
        image_archive: Some(archive),
        config_drive,
        management: None,
        nics: Vec::new(),
        status_socket: status_socket.clone(),
        serial_socket: None,
    };
    let base = base_image(&host.images_dir);
    start_domain(&host.qemu, domain, move |conn, domain| {
        libvirt::create_overlay(conn, &base, &domain.disk, None)?;
        libvirt::create_blank_qcow2(conn, &domain.docker_disk, STORE_DISK_GB)
    })
    .await?;
    let vm = store.vm();
    let stream = status::connect(&status_socket, &vm).await?;
    time::timeout(STORE_VM_LOAD, status::read_status(stream, &vm))
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

/// Write the archive of image `image` from the server's Docker to `path`, padded with zeros to
/// whole sectors so QEMU serves all of it as a disk.
///
/// # Errors
///
/// Fails when Docker cannot export the image or the archive cannot be written.
async fn write_image_archive(host: &NodeVmHost, image: &str, path: &str) -> Result<()> {
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
    fs::set_permissions(path, fs::Permissions::from_mode(ARCHIVE_MODE))
        .with_context(|| format!("Cannot set the mode of {path}"))
}

/// Remove the VM that makes `store`, its disk, config drive and image archive, and with
/// `delete_partial` the store's partial volume, where they are.
async fn remove_store_vm(host: &NodeVmHost, store: &Store, delete_partial: bool) -> Result<()> {
    let qemu = Arc::clone(&host.qemu);
    let vm = store.vm();
    let mut volumes = vec![store.vm_disk(), store.vm_config_drive(), store.archive()];
    if delete_partial {
        volumes.push(store.partial());
    }
    task::spawn_blocking(move || {
        let conn = qemu.connect()?;
        libvirt::remove_domain(&conn, &vm)?;
        for volume in &volumes {
            libvirt::delete_disk(&conn, volume)?;
        }
        Ok(())
    })
    .await
    .context("The Docker store VM removal panicked")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_vm_030_a_store_is_named_by_the_images_id() {
        let store = Store::of("sha256:0123456789abcdef0123456789abcdef").unwrap();
        assert_eq!(store.volume(), "flatcar-docker-0123456789abcdef.qcow2");
        assert_eq!(store.vm(), "sherpa-flatcar-docker-0123456789abcdef");
        assert!(Store::of("0123456789abcdef0123").is_none());
        assert!(Store::of("sha256:0123").is_none());
        assert!(Store::of("sha256:01234567../../etc/x").is_none());
    }
}
