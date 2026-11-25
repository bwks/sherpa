//! The read-only root disk a microVM boots a container image from, built once per image.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::Duration;

use bollard::Docker;
use bollard::errors::Error as DockerError;
use bollard::models::ContainerCreateBody;
use bollard::query_parameters::{CreateContainerOptionsBuilder, RemoveContainerOptionsBuilder};
use futures_util::StreamExt;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tracing::instrument;

use super::error::MicroVmError;
use super::{HYPERVISOR_BINARY, INITRAMFS_FILE, KERNEL_FILE, MICROVM_DIR, io_error, microvm_file};

const IMAGES_DIR: &str = "images";
const BUILD_OK: &str = "sherpa-init: BUILD-OK";
const BUILD_CMDLINE: &str = "console=ttyS0 sherpa.mode=build panic=-1 quiet";
const BUILD_MEMORY: &str = "size=1024M";
const BUILD_TIMEOUT: Duration = Duration::from_mins(30);
const MIB: u64 = 1024 * 1024;
const DISK_HEADROOM_BYTES: u64 = 512 * MIB;

/// One base disk is built at a time: a build is mostly disk I/O.
static BUILDS: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// The size of the disk an image archive of `tar_bytes` unpacks into: a quarter more, for
/// filesystem metadata, and a fixed headroom, rounded up to a MiB.
fn disk_bytes(tar_bytes: u64) -> u64 {
    (tar_bytes + tar_bytes / 4 + DISK_HEADROOM_BYTES).div_ceil(MIB) * MIB
}

/// The base disk of `image`, building it when there is none: Docker exports the image's
/// filesystem to an archive, and a builder microVM formats a disk and unpacks the archive into
/// it as root, keeping owners, modes and file capabilities. Nothing privileged runs on the
/// host. Returns the disk's path, `images/<image id>.raw`.
///
/// # Errors
///
/// Fails when Docker cannot inspect or export the image, or the builder does not finish.
#[instrument(skip(docker))]
pub async fn ensure_base_disk(docker: &Docker, image: &str) -> Result<PathBuf, MicroVmError> {
    let docker_error = |action| {
        move |source| MicroVmError::Docker {
            action,
            image: image.to_owned(),
            source,
        }
    };
    let inspect = docker
        .inspect_image(image)
        .await
        .map_err(docker_error("inspect"))?;
    let id = inspect
        .id
        .ok_or_else(|| MicroVmError::ImageWithoutId {
            image: image.to_owned(),
        })?
        .trim_start_matches("sha256:")
        .to_owned();
    let images_dir = Path::new(MICROVM_DIR).join(IMAGES_DIR);
    let disk = images_dir.join(format!("{id}.raw"));
    if disk.exists() {
        return Ok(disk);
    }
    let _build = BUILDS.lock().await;
    if disk.exists() {
        return Ok(disk);
    }
    fs::create_dir_all(&images_dir).map_err(io_error("create", &images_dir))?;
    let archive = images_dir.join(format!("{id}.tar.partial"));
    let partial = images_dir.join(format!("{id}.raw.partial"));
    let log = images_dir.join(format!("{id}.build.log"));
    let built = build(docker, image, &id, &archive, &partial, &log).await;
    let _ = fs::remove_file(&archive);
    if let Err(error) = built {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    fs::rename(&partial, &disk).map_err(io_error("rename", &partial))?;
    let _ = fs::remove_file(&log);
    tracing::info!(%image, disk = %disk.display(), "MicroVM base disk built");
    Ok(disk)
}

/// Export `image` to `archive`, then unpack it into the disk `partial` in a builder microVM
/// that logs to `log`.
async fn build(
    docker: &Docker,
    image: &str,
    id: &str,
    archive: &Path,
    partial: &Path,
    log: &Path,
) -> Result<(), MicroVmError> {
    let tar_bytes = export_image(docker, image, id, archive).await?;
    let disk = File::create(partial).map_err(io_error("create", partial))?;
    disk.set_len(disk_bytes(tar_bytes))
        .map_err(io_error("size", partial))?;
    drop(disk);

    let mut builder = Command::new(microvm_file(HYPERVISOR_BINARY));
    builder
        .arg("--kernel")
        .arg(microvm_file(KERNEL_FILE))
        .arg("--initramfs")
        .arg(microvm_file(INITRAMFS_FILE))
        .args([
            "--cmdline",
            BUILD_CMDLINE,
            "--cpus",
            "boot=2",
            "--memory",
            BUILD_MEMORY,
        ])
        .arg("--disk")
        .arg(format!("path={},image_type=raw", partial.display()))
        .arg(format!(
            "path={},readonly=on,image_type=raw",
            archive.display()
        ))
        .arg("--serial")
        .arg(format!("file={}", log.display()))
        .args(["--console", "off"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let failed = || MicroVmError::BaseDiskBuild {
        image: image.to_owned(),
        log: log.to_path_buf(),
    };
    let mut child = builder.spawn().map_err(|source| MicroVmError::Spawn {
        name: format!("builder for {image}"),
        source,
    })?;
    match timeout(BUILD_TIMEOUT, child.wait()).await {
        Ok(Ok(_)) => {}
        Ok(Err(_)) => return Err(failed()),
        Err(_) => {
            let _ = child.kill().await;
            return Err(MicroVmError::BaseDiskTimeout {
                image: image.to_owned(),
                log: log.to_path_buf(),
            });
        }
    }
    let output = fs::read_to_string(log).unwrap_or_default();
    if output.contains(BUILD_OK) {
        Ok(())
    } else {
        Err(failed())
    }
}

/// Write the filesystem of `image` to `archive` through a container that is created, exported
/// and removed, and return the archive's size.
async fn export_image(
    docker: &Docker,
    image: &str,
    id: &str,
    archive: &Path,
) -> Result<u64, MicroVmError> {
    let docker_error = |action| {
        move |source| MicroVmError::Docker {
            action,
            image: image.to_owned(),
            source,
        }
    };
    let name = format!("sherpa-export-{}", &id[..id.len().min(12)]);
    let options = CreateContainerOptionsBuilder::default().name(&name).build();
    let body = ContainerCreateBody {
        image: Some(image.to_owned()),
        cmd: Some(vec!["/sherpa-export".to_owned()]),
        ..Default::default()
    };
    docker
        .create_container(Some(options), body)
        .await
        .map_err(docker_error("create an export container"))?;
    let written = write_export(docker, &name, archive)
        .await
        .map_err(docker_error("export"));
    let remove = RemoveContainerOptionsBuilder::default().force(true).build();
    if let Err(error) = docker.remove_container(&name, Some(remove)).await {
        tracing::warn!(container = %name, %error, "Cannot remove the export container");
    }
    let written = written?;
    written.map_err(io_error("write", archive))
}

/// Stream container `name`'s export into `archive`; the outer error is Docker's, the inner
/// one the file's.
async fn write_export(
    docker: &Docker,
    name: &str,
    archive: &Path,
) -> Result<io::Result<u64>, DockerError> {
    let mut file = match File::create(archive) {
        Ok(file) => file,
        Err(error) => return Ok(Err(error)),
    };
    let mut stream = docker.export_container(name);
    let mut total = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if let Err(error) = file.write_all(&chunk) {
            return Ok(Err(error));
        }
        total += chunk.len() as u64;
    }
    Ok(file.sync_all().map(|()| total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privileged_microvm_007_a_base_disk_has_room_for_its_image_and_is_whole_mib() {
        let tar = 2_576_049_152;
        let disk = disk_bytes(tar);
        assert!(disk > tar + tar / 5);
        assert_eq!(disk % MIB, 0);
    }
}
