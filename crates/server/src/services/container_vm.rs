//! Container nodes whose models need root-like privileges run as the root filesystem of their
//! own QEMU guest, so those privileges reach the guest's kernel, not the server's. Docker
//! unpacks the image once per image ID; libvirt boots a small kernel and an init that mounts
//! it over virtio-fs and runs the image's entrypoint. See `docs/architecture/CONTAINER_VM.md`.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::iter;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use bollard::Docker;
use bollard::secret::ContainerStateStatusEnum;
use libvirt::{ContainerVm, ContainerVmNic, Qemu};
use tokio::task;
use tracing::instrument;

use shared::data::ModelKind;
use shared::konst::{CONTAINER_DISK_NAME, KVM_OUI};
use shared::util;

use crate::konst::{NODE_CONFIG_DIR_MODE, NODE_CONFIG_FILE_MODE, SHERPA_MANAGEMENT_NETWORK_NAME};
use crate::services::console;
use crate::services::node_ops::shell_quote;
use crate::services::p2p_links::ContainerVeth;

/// Where the guest kernel and initramfs are installed.
const BOOT_DIR: &str = "/opt/sherpa/vmboot";
/// Where Docker unpacks each image's root filesystem, by image ID.
const ROOTFS_STORE: &str = "/opt/sherpa/vmroot";
/// A read-only bind mount of `ROOTFS_STORE`, which virtiofsd serves from, so no guest can
/// change a root filesystem another node shares.
const ROOTFS_VIEW: &str = "/opt/sherpa/vmroot-ro";
/// The folder in a node's lab folder that is its guest's `node` share.
const NODE_SHARE: &str = "vm";
/// How many hex digits of the image ID name its root filesystem.
const ROOTFS_ID_LEN: usize = 16;
/// The management interface's name in the guest, as Docker names it in a container.
const MANAGEMENT_INTERFACE: &str = "eth0";
/// Memory and vCPUs of a guest whose node records none.
const DEFAULT_MEMORY_MIB: u64 = 4096;
const DEFAULT_VCPUS: u8 = 2;
/// The size of a node's disk, which holds the guest's changes to its root filesystem.
const DISK_BYTES: u64 = 8 << 30;
/// Docker's `/dev/shm` size, used when a node sets none.
const DEFAULT_SHM_BYTES: i64 = 64 * 1024 * 1024;
/// The search path a container gets from Docker when its image sets none.
const DEFAULT_PATH: &str = "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// What a container VM is started from: the node's container settings, as Docker would get
/// them, and its P2p ends.
pub(crate) struct ContainerVmStart<'a> {
    pub(crate) lab_id: &'a str,
    pub(crate) lab_dir: &'a str,
    pub(crate) node_name: &'a str,
    pub(crate) node_index: u16,
    pub(crate) model_kind: ModelKind,
    pub(crate) image: &'a str,
    pub(crate) env_vars: &'a [String],
    pub(crate) commands: &'a [String],
    pub(crate) volumes: &'a [String],
    pub(crate) shm_size: Option<i64>,
    pub(crate) memory_mib: Option<u64>,
    pub(crate) cpu_count: Option<u8>,
    pub(crate) p2p: &'a [&'a ContainerVeth],
}

/// A volume as the guest mounts it: a file or folder in the node share, where it goes, and
/// whether it is read-only.
#[derive(Debug, PartialEq, Eq)]
struct GuestMount {
    share_name: String,
    target: String,
    read_only: bool,
}

/// The domain name of node `node_name`'s container VM in lab `lab_id`: its container's name.
pub(crate) fn domain_name(node_name: &str, lab_id: &str) -> String {
    format!("{node_name}-{lab_id}")
}

/// The storage pool volume of container VM `domain` that holds its changes to the image.
fn disk_name(domain: &str) -> String {
    format!("{domain}-{CONTAINER_DISK_NAME}")
}

/// Unpack the node's image if no node has yet, write its share, make the node's disk unless it
/// has one, and define and start its domain with every P2p end as a tap named like the container's host veth, and join each
/// bridge end's tap to its bridge.
///
/// # Errors
///
/// Fails when Docker cannot inspect or unpack the image, the share cannot be written, or
/// libvirt cannot define or start the domain.
#[instrument(skip(docker, qemu, start), fields(lab_id = %start.lab_id, node_name = %start.node_name))]
pub(crate) async fn start_container_vm(
    docker: &Docker,
    qemu: &Arc<Qemu>,
    start: &ContainerVmStart<'_>,
) -> Result<()> {
    let image = container::image_run_config(docker, start.image).await?;
    let rootfs_id = rootfs_id(&image.id)?;
    ensure_rootfs(docker, start, &rootfs_id).await?;

    let share = PathBuf::from(format!(
        "{}/{}/{NODE_SHARE}",
        start.lab_dir, start.node_name
    ));
    let management_mac = util::random_mac(KVM_OUI);
    let mut interfaces = vec![(management_mac.clone(), MANAGEMENT_INTERFACE.to_string())];
    let mut nics = Vec::with_capacity(start.p2p.len());
    for veth in start.p2p {
        let mac = util::random_mac(KVM_OUI);
        interfaces.push((
            mac.clone(),
            util::container_interface_name(start.model_kind, veth.interface_idx)?,
        ));
        nics.push(ContainerVmNic {
            tap: veth.host_veth.clone(),
            mac,
            up: !veth.admin_down,
        });
    }
    write_share(&share, start, &image, &interfaces)?;

    let vm = ContainerVm {
        name: domain_name(start.node_name, start.lab_id),
        memory_mib: start.memory_mib.unwrap_or(DEFAULT_MEMORY_MIB),
        vcpus: start.cpu_count.unwrap_or(DEFAULT_VCPUS),
        kernel: format!("{BOOT_DIR}/vmlinuz"),
        initrd: format!("{BOOT_DIR}/initramfs.img"),
        rootfs_dir: format!("{ROOTFS_VIEW}/{rootfs_id}"),
        node_dir: share.display().to_string(),
        disk: String::new(),
        management_network: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{}", start.lab_id),
        management_mac,
        nics,
        serial_socket: console::console_sockets(start.lab_id, start.node_index)?.serial,
        serial_log: format!(
            "/var/log/libvirt/qemu/{}-serial.log",
            domain_name(start.node_name, start.lab_id)
        ),
    };
    let qemu = Arc::clone(qemu);
    task::spawn_blocking(move || {
        let conn = qemu.connect()?;
        let mut vm = vm;
        vm.disk = libvirt::ensure_raw_volume(&conn, &disk_name(&vm.name), DISK_BYTES)?;
        libvirt::create_container_vm(&conn, &vm)
    })
    .await
    .context("The container VM start panicked")??;
    for veth in start.p2p {
        if let Some(bridge) = &veth.bridge {
            network::enslave_to_bridge(&veth.host_veth, bridge).await?;
        }
    }
    Ok(())
}

/// The status of container VM `domain` as Docker would report its container: running while
/// the domain runs, exited when it is stopped, `None` when there is no such domain.
///
/// # Errors
///
/// Fails when libvirt cannot be reached or the domain's state read.
#[instrument(skip(qemu), level = "debug")]
pub(crate) async fn domain_status(
    qemu: &Arc<Qemu>,
    domain: &str,
) -> Result<Option<ContainerStateStatusEnum>> {
    let qemu = Arc::clone(qemu);
    let domain = domain.to_string();
    let state = task::spawn_blocking(move || {
        let conn = qemu.connect()?;
        libvirt::domain_state(&conn, &domain)
    })
    .await
    .context("The container VM state read panicked")??;
    Ok(state.map(|running| {
        if running {
            ContainerStateStatusEnum::RUNNING
        } else {
            ContainerStateStatusEnum::EXITED
        }
    }))
}

/// Stop and undefine container VM `domain` and delete its disk; what is already gone counts as
/// removed.
///
/// # Errors
///
/// Fails when libvirt cannot be reached, the domain cannot be stopped or undefined, or its
/// disk cannot be deleted.
#[instrument(skip(qemu))]
pub(crate) async fn remove_container_vm(qemu: &Arc<Qemu>, domain: &str) -> Result<()> {
    let qemu = Arc::clone(qemu);
    let domain = domain.to_string();
    task::spawn_blocking(move || {
        let conn = qemu.connect()?;
        libvirt::remove_domain(&conn, &domain)?;
        libvirt::delete_volume_if_present(&conn, &disk_name(&domain))
    })
    .await
    .context("The container VM removal panicked")?
}

/// The name of an image's root filesystem in the store: the first hex digits of its ID.
fn rootfs_id(image_id: &str) -> Result<String> {
    let hex = image_id.strip_prefix("sha256:").unwrap_or(image_id);
    hex.get(..ROOTFS_ID_LEN)
        .filter(|id| id.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_string)
        .ok_or_else(|| anyhow!("Image ID '{image_id}' is not a sha256 digest"))
}

/// Unpack the image into the store under `rootfs_id`, unless it is there already. The unpack
/// goes to a folder of the node's own, renamed into place once complete, so a node never boots
/// a half-unpacked root.
async fn ensure_rootfs(
    docker: &Docker,
    start: &ContainerVmStart<'_>,
    rootfs_id: &str,
) -> Result<()> {
    let done = Path::new(ROOTFS_STORE).join(rootfs_id);
    if done.is_dir() {
        return Ok(());
    }
    let helper = format!("unpack-{}", domain_name(start.node_name, start.lab_id));
    let partial = Path::new(ROOTFS_STORE).join(format!("{rootfs_id}.{helper}"));
    tracing::info!(image = %start.image, %rootfs_id, "Unpacking the image for its container VM");
    container::unpack_image_rootfs(docker, start.image, &partial.display().to_string(), &helper)
        .await?;
    match fs::rename(&partial, &done) {
        Ok(()) => Ok(()),
        Err(error) if done.is_dir() => {
            tracing::warn!(%rootfs_id, %error, "Another node unpacked the image first");
            Ok(())
        }
        Err(error) => Err(error)
            .with_context(|| format!("Failed to move the unpacked image to {}", done.display())),
    }
}

/// The guest's interface names by MAC, one `mac name` per line, the MAC in the lowercase form
/// the guest kernel reports.
fn interfaces_file(interfaces: &[(String, String)]) -> String {
    let mut file = String::new();
    for (mac, name) in interfaces {
        let _ = writeln!(file, "{} {name}", mac.to_ascii_lowercase());
    }
    file
}

/// The guest's mounts, one `share-name target mode` per line, the mode `ro` or `rw`.
fn mounts_file(mounts: &[GuestMount]) -> String {
    let mut file = String::new();
    for mount in mounts {
        let mode = if mount.read_only { "ro" } else { "rw" };
        let _ = writeln!(file, "{} {} {mode}", mount.share_name, mount.target);
    }
    file
}

/// Write the node share: the start script, the interface names by MAC, the mounts and their
/// sources, the hostname and the `/dev/shm` size.
fn write_share(
    share: &Path,
    start: &ContainerVmStart<'_>,
    image: &container::ImageRunConfig,
    interfaces: &[(String, String)],
) -> Result<()> {
    if share.exists() {
        fs::remove_dir_all(share)
            .with_context(|| format!("Failed to clear the node share {}", share.display()))?;
    }
    util::create_dir_with_permissions(share, NODE_CONFIG_DIR_MODE)?;
    let mounts = guest_mounts(start.volumes)?;
    for (volume, mount) in start.volumes.iter().zip(&mounts) {
        let source = volume.split(':').next().unwrap_or_default();
        copy_into_share(Path::new(source), &share.join(&mount.share_name))?;
    }
    let files = [
        (
            "start.sh",
            start_script(image, start.env_vars, start.commands),
        ),
        ("interfaces", interfaces_file(interfaces)),
        ("mounts", mounts_file(&mounts)),
        ("hostname", format!("{}\n", start.node_name)),
        (
            "shm_size",
            start.shm_size.unwrap_or(DEFAULT_SHM_BYTES).to_string(),
        ),
    ];
    for (name, content) in files {
        util::write_file_with_permissions(
            &share.join(name),
            content.as_bytes(),
            NODE_CONFIG_FILE_MODE,
        )?;
    }
    Ok(())
}

/// The guest mounts of Docker volume specs `src:dst[:ro]`, each source named `volN` in the
/// share.
fn guest_mounts(volumes: &[String]) -> Result<Vec<GuestMount>> {
    volumes
        .iter()
        .enumerate()
        .map(|(index, volume)| {
            let mut parts = volume.split(':');
            let (Some(_), Some(target)) = (parts.next(), parts.next()) else {
                return Err(anyhow!("Volume '{volume}' is not src:dst"));
            };
            Ok(GuestMount {
                share_name: format!("vol{index}"),
                target: target.to_string(),
                read_only: parts.next() == Some("ro"),
            })
        })
        .collect()
}

/// Copy file or folder `source` to `dest`, folders recursively.
fn copy_into_share(source: &Path, dest: &Path) -> Result<()> {
    let metadata = fs::metadata(source)
        .with_context(|| format!("Failed to read volume source {}", source.display()))?;
    if !metadata.is_dir() {
        fs::copy(source, dest)
            .with_context(|| format!("Failed to copy {} into the node share", source.display()))?;
        return Ok(());
    }
    util::create_dir_with_permissions(dest, NODE_CONFIG_DIR_MODE)?;
    for entry in fs::read_dir(source)
        .with_context(|| format!("Failed to read volume folder {}", source.display()))?
    {
        let entry = entry?;
        copy_into_share(&entry.path(), &dest.join(entry.file_name()))?;
    }
    Ok(())
}

/// The script the guest init runs as the image's process: the image's environment with the
/// node's over it, its working folder, then its entrypoint and command, `commands` replacing
/// the image's command as Docker's `Cmd` does.
fn start_script(
    image: &container::ImageRunConfig,
    env_vars: &[String],
    commands: &[String],
) -> String {
    let mut env: HashMap<&str, &str> = HashMap::new();
    let mut order = Vec::new();
    for var in iter::once(DEFAULT_PATH)
        .chain(image.env.iter().map(String::as_str))
        .chain(env_vars.iter().map(String::as_str))
    {
        let (key, value) = var.split_once('=').unwrap_or((var, ""));
        if env.insert(key, value).is_none() {
            order.push(key);
        }
    }
    let mut script = String::new();
    for key in order {
        if key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !key.is_empty() {
            let _ = writeln!(script, "export {key}={}", shell_quote(env[key]));
        }
    }
    let workdir = if image.working_dir.is_empty() {
        "/"
    } else {
        &image.working_dir
    };
    let _ = writeln!(script, "cd {}", shell_quote(workdir));
    let cmd = if commands.is_empty() {
        &image.cmd
    } else {
        commands
    };
    let argv: Vec<String> = image
        .entrypoint
        .iter()
        .chain(cmd)
        .map(|arg| shell_quote(arg))
        .collect();
    let _ = writeln!(script, "exec {}", argv.join(" "));
    script
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> container::ImageRunConfig {
        container::ImageRunConfig {
            id: "sha256:0096fe3ebcafabb7aaaa".to_string(),
            entrypoint: vec!["/tini".to_string(), "--".to_string()],
            cmd: vec!["/bin/bash".to_string()],
            env: vec!["LANG=C.UTF-8".to_string(), "SRLINUX=0".to_string()],
            working_dir: String::new(),
        }
    }

    #[test]
    fn container_vm_007_the_start_script_runs_the_entrypoint_with_the_nodes_command() {
        let script = start_script(
            &image(),
            &["SRLINUX=1".to_string()],
            &["bash".to_string(), "/opt/srlinux/bin/sr_linux".to_string()],
        );
        assert!(script.contains("export LANG='C.UTF-8'\n"));
        assert!(script.contains("export SRLINUX='1'\n"));
        assert!(!script.contains("SRLINUX='0'"));
        assert!(script.contains("export PATH="));
        assert!(script.ends_with("cd '/'\nexec '/tini' '--' 'bash' '/opt/srlinux/bin/sr_linux'\n"));
    }

    #[test]
    fn container_vm_008_without_a_node_command_the_images_command_runs() {
        let script = start_script(&image(), &[], &[]);
        assert!(script.ends_with("exec '/tini' '--' '/bin/bash'\n"));
    }

    #[test]
    fn container_vm_009_a_value_with_a_quote_stays_one_shell_word() {
        let script = start_script(&image(), &["MSG=it's".to_string()], &[]);
        assert!(script.contains(r"export MSG='it'\''s'"));
    }

    #[test]
    fn container_vm_010_volumes_become_share_mounts_with_their_mode() {
        let mounts = guest_mounts(&[
            "/lab/r1/r1.conf:/mnt/sherpa-ztp/startup-config:ro".to_string(),
            "/lab/r1/r1.json:/etc/opt/srlinux/config.json".to_string(),
        ])
        .unwrap();
        assert_eq!(
            mounts,
            vec![
                GuestMount {
                    share_name: "vol0".to_string(),
                    target: "/mnt/sherpa-ztp/startup-config".to_string(),
                    read_only: true,
                },
                GuestMount {
                    share_name: "vol1".to_string(),
                    target: "/etc/opt/srlinux/config.json".to_string(),
                    read_only: false,
                },
            ]
        );
    }

    #[test]
    fn container_vm_015_interface_names_are_keyed_by_the_lowercase_mac_the_guest_reports() {
        let interfaces = [
            ("52:54:00:60:16:80".to_string(), "eth0".to_string()),
            ("52:54:00:CD:EE:10".to_string(), "e1-1".to_string()),
        ];
        assert_eq!(
            interfaces_file(&interfaces),
            "52:54:00:60:16:80 eth0\n52:54:00:cd:ee:10 e1-1\n"
        );
    }

    #[test]
    fn container_vm_011_the_rootfs_is_named_by_the_image_id() {
        assert_eq!(
            rootfs_id("sha256:0096fe3ebcafabb7aaaa").unwrap(),
            "0096fe3ebcafabb7"
        );
        assert!(rootfs_id("sha256:../../etc").is_err());
    }
}
