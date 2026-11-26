//! The Docker store of an image for node VMs: a disk that holds Docker's data with the image
//! loaded, made once per image by a Flatcar VM of its own. Each node VM of the image boots on an
//! overlay of it, so starting a node copies nothing and the image is on the server's disk once.

use std::fs;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use bollard::Docker;
use dashmap::DashMap;
use libvirt::{NodeVm, Qemu};
use tokio::sync::Mutex;
use tokio::task;
use tracing::instrument;

use shared::konst::SHERPA_STORAGE_POOL_PATH;
use shared::util;

use crate::konst::NODE_CONFIG_DIR_MODE;

use super::ignition::{DockerTls, store_ignition_json};
use super::{
    NODE_VM_IMAGE_DIR, NodeVmCerts, base_image, base_kernel, config_drive_name, disk_name,
    kernel_cmdline, kernel_usr_hash, wait_for_docker, write_config_drive,
};
use container::NodeVmTls;

/// The size of a Docker store: the image, and what each node of it writes.
const STORE_DISK_GB: u16 = 32;
/// The hex digits of the image ID a store is named by.
const STORE_ID_LEN: usize = 16;
/// The memory of the VM that makes a store.
const STORE_VM_MEMORY_MIB: u64 = 2048;
/// The vCPUs of the VM that makes a store.
const STORE_VM_VCPUS: u8 = 2;
/// How long the VM that made a store has to shut down.
const STORE_VM_SHUTDOWN: Duration = Duration::from_secs(120);

/// One lock per store, so the nodes of one image wait for the one VM that makes it.
static STORE_LOCKS: OnceLock<DashMap<String, Arc<Mutex<()>>>> = OnceLock::new();

/// The name of the Docker store of image ID `image_id` (`sha256:HEX`), or `None` when the ID
/// is not one.
fn store_name(image_id: &str) -> Option<String> {
    let hex = image_id.strip_prefix("sha256:")?;
    let id = hex.get(..STORE_ID_LEN)?;
    id.bytes()
        .all(|byte| byte.is_ascii_hexdigit())
        .then(|| format!("flatcar-docker-{id}.qcow2"))
}

/// The domain name of the VM that makes store `store`.
fn store_vm_name(store: &str) -> String {
    format!("sherpa-{}", store.trim_end_matches(".qcow2"))
}

/// The storage pool volume a store is made in, renamed to the store once it is complete.
fn partial_name(store: &str) -> String {
    format!("{store}.part")
}

/// The path of the Docker store of image `image`, made first when there is none: a disk whose
/// Docker has the image loaded, which nodes of the image boot overlays of. A store is named by
/// the image's ID, so an image imported again under the same name gets a store of its own.
///
/// # Errors
///
/// Fails when the server's Docker does not have the image, or the store cannot be made.
#[instrument(skip(host, qemu), level = "debug")]
pub(super) async fn ensure_docker_store(
    host: &Docker,
    qemu: &Arc<Qemu>,
    images_dir: &str,
    image: &str,
) -> Result<String> {
    let id = host
        .inspect_image(image)
        .await
        .with_context(|| format!("Cannot inspect image {image}"))?
        .id
        .with_context(|| format!("Image {image} has no ID"))?;
    let store = store_name(&id).with_context(|| format!("Image {image} has ID '{id}'"))?;
    let path = format!("{SHERPA_STORAGE_POOL_PATH}/{store}");
    let lock = Arc::clone(
        &STORE_LOCKS
            .get_or_init(DashMap::new)
            .entry(store.clone())
            .or_default(),
    );
    let _guard = lock.lock().await;
    if Path::new(&path).is_file() {
        return Ok(path);
    }
    tracing::info!(%image, %store, "Making the node VM Docker store of the image");
    let made = make_store(host, qemu, images_dir, image, &store).await;
    if let Err(removal) = remove_store_vm(qemu, &store, made.is_err()).await {
        tracing::warn!(%store, error = %removal, "Cannot remove the VM that made a Docker store");
    }
    made?;
    let partial = format!("{SHERPA_STORAGE_POOL_PATH}/{}", partial_name(&store));
    fs::rename(&partial, &path).with_context(|| format!("Cannot rename {partial} to {path}"))?;
    Ok(path)
}

/// Make store `store` of image `image`: boot a Flatcar VM with an empty disk as its Docker
/// data, copy the image into its Docker, and shut it down cleanly.
async fn make_store(
    host: &Docker,
    qemu: &Arc<Qemu>,
    images_dir: &str,
    image: &str,
    store: &str,
) -> Result<()> {
    remove_store_vm(qemu, store, true).await?;
    let device = store_vm_name(store);
    let certs = NodeVmCerts::issue(&device)?;
    let ignition = store_ignition_json(&DockerTls {
        ca: &certs.ca,
        server_cert: &certs.server_cert,
        server_key: &certs.server_key,
    })?;
    let stage = Path::new(images_dir)
        .join(NODE_VM_IMAGE_DIR)
        .join(format!(".{device}"));
    util::create_dir_with_permissions(&stage, NODE_CONFIG_DIR_MODE)?;
    let config_iso = write_config_drive(&stage, &device, &ignition);
    fs::remove_dir_all(&stage).with_context(|| format!("Cannot remove {}", stage.display()))?;
    let config_iso = config_iso?;
    let vm = NodeVm {
        name: device.clone(),
        memory_mib: STORE_VM_MEMORY_MIB,
        vcpus: STORE_VM_VCPUS,
        kernel: base_kernel(images_dir),
        cmdline: kernel_cmdline(&kernel_usr_hash(images_dir)?, true),
        disk: format!("{SHERPA_STORAGE_POOL_PATH}/{}", disk_name(&device)),
        docker_disk: format!("{SHERPA_STORAGE_POOL_PATH}/{}", partial_name(store)),
        config_iso,
        management: None,
        nics: Vec::new(),
        serial_socket: None,
    };
    let base = base_image(images_dir);
    {
        let qemu = Arc::clone(qemu);
        let vm = vm.clone();
        task::spawn_blocking(move || {
            let conn = qemu.connect()?;
            libvirt::create_overlay(&conn, &base, &vm.disk, None)?;
            libvirt::create_blank_qcow2(&conn, &vm.docker_disk, STORE_DISK_GB)?;
            libvirt::create_node_vm(&conn, &vm)
        })
        .await
        .context("The Docker store VM start panicked")??;
    }
    let tls = NodeVmTls {
        ca_pem: certs.ca,
        client_cert_pem: certs.client_cert,
        client_key_pem: certs.client_key,
        server_name: device.clone(),
    };
    let docker = wait_for_docker(qemu, &device, &tls).await?;
    container::copy_image_to_node_vm(host, &docker, image).await?;
    let qemu = Arc::clone(qemu);
    task::spawn_blocking(move || {
        let conn = qemu.connect()?;
        libvirt::shut_down_node_vm(&conn, &device, STORE_VM_SHUTDOWN)
    })
    .await
    .context("The Docker store VM shutdown panicked")?
}

/// Remove the VM that makes store `store`, its disk and config drive, and with
/// `delete_partial` the store's partial volume. What is already gone counts as removed.
async fn remove_store_vm(qemu: &Arc<Qemu>, store: &str, delete_partial: bool) -> Result<()> {
    let device = store_vm_name(store);
    let qemu = Arc::clone(qemu);
    let partial = partial_name(store);
    task::spawn_blocking(move || {
        let conn = qemu.connect()?;
        libvirt::remove_node_vm(&conn, &device)?;
        libvirt::delete_volume_if_present(&conn, &disk_name(&device))?;
        libvirt::delete_volume_if_present(&conn, &config_drive_name(&device))?;
        if delete_partial {
            libvirt::delete_volume_if_present(&conn, &partial)?;
        }
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("The Docker store VM removal panicked")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_vm_030_a_store_is_named_by_the_images_id() {
        assert_eq!(
            store_name("sha256:0123456789abcdef0123456789abcdef").as_deref(),
            Some("flatcar-docker-0123456789abcdef.qcow2")
        );
        assert_eq!(store_name("0123456789abcdef0123"), None);
        assert_eq!(store_name("sha256:0123"), None);
        assert_eq!(store_name("sha256:01234567../../etc/x"), None);
    }
}
