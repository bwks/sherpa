//! The boot disk of sandboxes: the base image with one change, the kernel arguments sandboxes
//! boot with, in Flatcar's OEM partition, so a sandbox boots with them from its first boot on.
//! It is made once per base image version: a Flatcar VM boots an overlay of the base image whose
//! Ignition sets only the kernel arguments, which Flatcar writes to the OEM partition and reboots
//! before anything else on the disk changes, and that reboot stops the VM. The overlay is then
//! copied with its base into a standalone disk, which sandboxes boot overlays of.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use askama::Template;
use tokio::task;
use tracing::instrument;

use shared::data::StatusKind;
use template::{SandboxBootDomainTemplate, boot_vm_ignition_json};

use super::base::SandboxBase;
use super::create::{DomainDisk, SandboxHost, pool_path, start_domain};
use super::guest::write_config_drive;
use crate::daemon::storage::restrict_dir;
use crate::konst::{CONSOLE_GROUP, CONSOLE_LAB_DIR_MODE, SHERPA_SANDBOX_RUN_DIR};

/// The memory of the VM that makes a boot disk.
const BOOT_VM_MEMORY_MIB: u64 = 2048;
/// The vCPUs of the VM that makes a boot disk.
const BOOT_VM_VCPUS: u8 = 2;
/// How long the VM that makes a boot disk has to boot and stop at its first reboot.
const BOOT_VM_STOP: Duration = Duration::from_mins(10);
/// The start of the name of a boot disk and of a partial one. A node's volume has no `.` before
/// its suffix, as a node name has none.
const BOOT_PREFIX: &str = "flatcar-sandbox.";
/// The start of the domain name of a boot disk's VM, and of its disk and config drive.
const BOOT_VM_PREFIX: &str = "sherpa-flatcar-sandbox.";
/// The end of a boot disk's volume name.
const BOOT_SUFFIX: &str = ".qcow2";
/// The end of a partial boot disk's volume name.
const PARTIAL_SUFFIX: &str = ".qcow2.part";

/// The names of the boot disk of one base image version and of what makes it.
pub(crate) struct BootDisk {
    version: String,
}

impl BootDisk {
    /// The boot disk of version `version` of the `flatcar_sandbox` image.
    pub(crate) fn of(version: &str) -> Self {
        Self {
            version: version.to_owned(),
        }
    }

    /// The boot disk's volume.
    pub(crate) fn volume(&self) -> String {
        format!("{BOOT_PREFIX}{}{BOOT_SUFFIX}", self.version)
    }

    /// The volume the VM's disk is copied into, renamed to the boot disk once it is complete.
    fn partial(&self) -> String {
        format!("{BOOT_PREFIX}{}{PARTIAL_SUFFIX}", self.version)
    }

    /// The domain name of the VM that makes the boot disk.
    fn vm(&self) -> String {
        format!("{BOOT_VM_PREFIX}{}", self.version)
    }

    /// The VM's disk, an overlay on the base image.
    fn vm_disk(&self) -> String {
        format!("{}-hdd.qcow2", self.vm())
    }

    /// The VM's config drive.
    fn config_drive(&self) -> String {
        format!("{}.iso", self.vm())
    }

    /// The folder in run folder `run_dir` the VM's config drive is staged in.
    fn stage(&self, run_dir: &Path) -> PathBuf {
        run_dir.join(format!(".{}", self.vm()))
    }
}

/// Whether pool volume `volume` is one a boot disk build makes and removes again, the VM's
/// disk or config drive or the partial boot disk, and not a boot disk.
pub(super) fn is_build_volume(volume: &str) -> bool {
    volume.starts_with(BOOT_VM_PREFIX)
        || (volume.starts_with(BOOT_PREFIX) && volume.ends_with(PARTIAL_SUFFIX))
}

/// Whether domain `name` is the VM of a boot disk build.
pub(super) fn is_build_domain(name: &str) -> bool {
    name.starts_with(BOOT_VM_PREFIX)
}

/// Whether `name`, in the sandbox run folder, is the staging folder of a boot disk build.
pub(super) fn is_build_file(name: &str) -> bool {
    name.starts_with(&format!(".{BOOT_VM_PREFIX}"))
}

/// The path of the boot disk of base image `base`, made first when there is none.
///
/// # Errors
///
/// Fails when the boot disk cannot be made.
#[instrument(skip(host, base), fields(version = %base.version), level = "debug")]
pub(super) async fn ensure_boot_disk(host: &SandboxHost, base: &SandboxBase) -> Result<String> {
    let boot = BootDisk::of(&base.version);
    let path = pool_path(&boot.volume());
    let lock = host.disk_locks.of(&boot.volume());
    let _guard = if let Ok(guard) = lock.try_lock() {
        guard
    } else {
        let _ = host.progress.send_status(
            format!(
                "Waiting for the sandbox boot disk of flatcar_sandbox {} to be made",
                base.version
            ),
            StatusKind::Waiting,
        );
        lock.lock().await
    };
    if Path::new(&path).is_file() {
        return Ok(path);
    }
    tracing::info!(version = %base.version, disk = %boot.volume(), "Making the sandbox boot disk");
    let _ = host.progress.send_status(
        format!(
            "Making the sandbox boot disk of flatcar_sandbox {}: a VM sets Flatcar's kernel \
             arguments, once per image version",
            base.version
        ),
        StatusKind::Progress,
    );
    let made = make_boot_disk(host, base, &boot).await;
    if let Err(removal) = remove_boot_vm(host, &boot, made.is_err()).await {
        tracing::warn!(disk = %boot.volume(), error = %removal, "Cannot remove the VM that made a sandbox boot disk");
    }
    made?;
    let partial = pool_path(&boot.partial());
    fs::rename(&partial, &path).with_context(|| format!("Cannot rename {partial} to {path}"))?;
    let _ = host.progress.send_status(
        format!(
            "Made the sandbox boot disk of flatcar_sandbox {}",
            base.version
        ),
        StatusKind::Done,
    );
    Ok(path)
}

/// Make `boot` from base image `base`: boot a Flatcar VM on an overlay of the base image whose
/// Ignition sets only the sandboxes' kernel arguments, wait until the reboot Flatcar makes to
/// apply them has stopped it, and copy its disk with the base image into the partial boot disk.
async fn make_boot_disk(host: &SandboxHost, base: &SandboxBase, boot: &BootDisk) -> Result<()> {
    remove_boot_vm(host, boot, true).await?;
    restrict_dir(
        Path::new(SHERPA_SANDBOX_RUN_DIR),
        CONSOLE_LAB_DIR_MODE,
        Some(CONSOLE_GROUP),
    )?;
    let config_drive = write_config_drive(
        &boot.stage(Path::new(SHERPA_SANDBOX_RUN_DIR)),
        &boot.config_drive(),
        &boot_vm_ignition_json()?,
        None,
        &[],
    )?;
    let vm = boot.vm();
    let domain = SandboxBootDomainTemplate {
        name: vm.clone(),
        memory_mib: BOOT_VM_MEMORY_MIB,
        vcpus: BOOT_VM_VCPUS,
        disk: pool_path(&boot.vm_disk()),
        config_drive,
    };
    let xml = domain
        .render()
        .with_context(|| format!("Cannot render the domain of VM {vm}"))?;
    let disks = vec![DomainDisk::Overlay {
        base: base.disk.clone(),
        path: domain.disk.clone(),
    }];
    start_domain(&host.qemu, &vm, xml, disks).await?;
    let qemu = Arc::clone(&host.qemu);
    let vm_disk = pool_path(&boot.vm_disk());
    let partial = pool_path(&boot.partial());
    task::spawn_blocking(move || {
        let conn = qemu.connect()?;
        libvirt::wait_until_stopped(&conn, &vm, BOOT_VM_STOP).with_context(|| {
            format!(
                "VM {vm} did not set Flatcar's kernel arguments within {} seconds",
                BOOT_VM_STOP.as_secs()
            )
        })?;
        libvirt::flatten_disk(&conn, &vm_disk, &partial)
    })
    .await
    .context("The sandbox boot disk VM wait panicked")?
}

/// Remove the VM that makes `boot`, its disk and config drive, and with `delete_partial` the
/// partial boot disk, where they are.
async fn remove_boot_vm(host: &SandboxHost, boot: &BootDisk, delete_partial: bool) -> Result<()> {
    let qemu = Arc::clone(&host.qemu);
    let vm = boot.vm();
    let mut volumes = vec![boot.vm_disk(), boot.config_drive()];
    if delete_partial {
        volumes.push(boot.partial());
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
    .context("The sandbox boot disk VM removal panicked")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_073_a_boot_disk_is_named_by_the_base_images_version_and_no_nodes_volume_is_its() {
        let boot = BootDisk::of("4757.2.4");
        assert_eq!(boot.volume(), "flatcar-sandbox.4757.2.4.qcow2");
        assert_eq!(boot.vm(), "sherpa-flatcar-sandbox.4757.2.4");
        for volume in [boot.vm_disk(), boot.config_drive(), boot.partial()] {
            assert!(is_build_volume(&volume), "{volume}");
        }
        assert!(!is_build_volume(&boot.volume()));
        assert!(is_build_domain(&boot.vm()));
        assert!(is_build_file(".sherpa-flatcar-sandbox.4757.2.4"));
        for node_volume in [
            "flatcar-sandbox-abc12345-hdd.qcow2",
            "sherpa-flatcar-sandbox-abc12345-hdd.qcow2",
            "sherpa-flatcar-sandbox-abc12345.iso",
        ] {
            assert!(!is_build_volume(node_volume), "{node_volume}");
        }
        assert!(!is_build_domain("sherpa-flatcar-sandbox-abc12345"));
    }
}
