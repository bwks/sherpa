//! Remove what unfinished Docker store builds left.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use libvirt::Qemu;
use tokio::task;
use tracing::instrument;

use super::store;

/// Remove the VMs, volumes, staging folders and status sockets that unfinished store builds left.
#[instrument(skip(qemu))]
pub(crate) async fn remove_unfinished_builds(qemu: &Arc<Qemu>, run_dir: &Path) -> Result<()> {
    let qemu = Arc::clone(qemu);
    let run_dir = run_dir.to_owned();
    let removed = task::spawn_blocking(move || {
        let conn = qemu.connect()?;
        let mut removed = Vec::new();
        for name in libvirt::list_domain_names(&conn)? {
            if store::is_build_domain(&name) {
                libvirt::remove_domain(&conn, &name)?;
                removed.push(name);
            }
        }
        for volume in libvirt::list_pool_volumes(&conn)? {
            if store::is_build_volume(&volume) {
                libvirt::delete_disk(&conn, &volume)?;
                removed.push(volume);
            }
        }
        removed.extend(remove_build_files(&run_dir)?);
        Ok::<_, anyhow::Error>(removed)
    })
    .await
    .context("The sandbox build sweep panicked")??;
    if !removed.is_empty() {
        tracing::warn!(?removed, "Removed what unfinished sandbox disk builds left");
    }
    Ok(())
}

/// Remove the build staging folders and status sockets in `dir` and return their paths.
fn remove_build_files(dir: &Path) -> Result<Vec<String>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("Cannot read {}", dir.display()));
        }
    };
    let mut removed = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("Cannot read {}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !store::is_build_file(&name) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            fs::remove_dir_all(&path)
                .with_context(|| format!("Cannot remove {}", path.display()))?;
        } else {
            fs::remove_file(&path).with_context(|| format!("Cannot remove {}", path.display()))?;
        }
        removed.push(path.display().to_string());
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use libvirt::VolumeOwner;
    use nix::unistd::{getegid, geteuid};
    use tempfile::TempDir;
    use virt::connect::Connect;
    use virt::domain::Domain;

    use super::*;
    use crate::services::sandbox::create::pool_path;

    /// Define a domain named `name` that never runs.
    fn define_domain(conn: &Connect, name: &str) {
        Domain::define_xml(
            conn,
            &format!(
                "<domain type='kvm'><name>{name}</name><memory unit='MiB'>64</memory>\
                 <os><type arch='x86_64'>hvm</type></os></domain>"
            ),
        )
        .unwrap();
    }

    #[tokio::test]
    #[ignore = "Requires libvirt and sherpa-pool, and no sandbox disk being made; run as root"]
    async fn sandbox_074_the_start_sweep_keeps_a_nodes_domain_and_volumes_named_like_a_builds() {
        let qemu = Arc::new(Qemu::default());
        let conn = qemu.connect().unwrap();
        let node = "sherpa-store-st074074";
        let volumes = [
            format!("{node}-hdd.qcow2"),
            format!("{node}-docker.qcow2"),
            format!("{node}.iso"),
        ];
        let owner = VolumeOwner {
            uid: geteuid().as_raw(),
            gid: getegid().as_raw(),
        };
        for volume in &volumes {
            libvirt::create_blank_qcow2(&conn, &pool_path(volume), 1, owner).unwrap();
        }
        define_domain(&conn, node);
        let run = TempDir::new().unwrap();

        let swept = remove_unfinished_builds(&qemu, run.path()).await;

        let present = libvirt::list_pool_volumes(&conn).unwrap();
        let domains = libvirt::list_domain_names(&conn).unwrap();
        libvirt::remove_domain(&conn, node).unwrap();
        for volume in volumes.iter().filter(|volume| present.contains(volume)) {
            libvirt::delete_disk(&conn, volume).unwrap();
        }
        swept.unwrap();
        for volume in &volumes {
            assert!(present.contains(volume), "{volume}");
        }
        assert!(domains.iter().any(|domain| domain == node));
    }
}
