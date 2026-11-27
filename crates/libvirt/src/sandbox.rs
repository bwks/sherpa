//! Sandbox domains and disks: waiting for a domain to stop, and copying a disk with its
//! backing chain into a standalone one.

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use askama::Template;
use shared::konst::SHERPA_STORAGE_POOL;
use tracing::instrument;
use virt::connect::Connect;
use virt::domain::Domain;
use virt::storage_pool::StoragePool;
use virt::storage_vol::StorageVol;

use crate::error::LibvirtError;
use crate::qcow2::read_qcow2_metadata;
use crate::storage::{Qcow2Volume, new_volume_name, refresh_pool};

/// The pause between two checks of whether a domain has stopped.
const STOPPED_POLL: Duration = Duration::from_secs(1);

/// Wait until domain `name` has stopped, for up to `timeout`.
///
/// # Errors
///
/// Fails when libvirt cannot find the domain or read whether it runs, or it still runs after
/// `timeout`.
#[instrument(level = "debug", skip(conn))]
pub fn wait_until_stopped(conn: &Connect, name: &str, timeout: Duration) -> Result<()> {
    let domain = Domain::lookup_by_name(conn, name)
        .with_context(|| format!("Cannot look up domain {name}"))?;
    let started = Instant::now();
    while domain
        .is_active()
        .with_context(|| format!("Cannot read whether domain {name} runs"))?
    {
        if started.elapsed() >= timeout {
            return Err(LibvirtError::StopTimedOut {
                name: name.to_owned(),
                seconds: timeout.as_secs(),
            }
            .into());
        }
        thread::sleep(STOPPED_POLL);
    }
    Ok(())
}

/// Copy pool volume `src` with what its backing chain holds into the new standalone QCOW2
/// volume `dst`, created exclusively and readable by the group, as sherpad builds overlays on it.
///
/// # Errors
///
/// Fails when `dst` exists, is relative or is not a UTF-8 file name, libvirt cannot find `src`
/// or copy it, or the copy still has a backing file.
#[instrument(level = "debug", skip(conn))]
pub fn flatten_disk(conn: &Connect, src: &str, dst: &str) -> Result<()> {
    let start = Instant::now();
    let name = new_volume_name(dst)?;
    let pool = StoragePool::lookup_by_name(conn, SHERPA_STORAGE_POOL)
        .with_context(|| format!("Cannot find storage pool '{SHERPA_STORAGE_POOL}'"))?;
    refresh_pool(&pool)?;
    let source = StorageVol::lookup_by_path(conn, src)
        .with_context(|| format!("Cannot find volume {src}"))?;
    let capacity = source
        .get_info()
        .with_context(|| format!("Cannot read the size of volume {src}"))?
        .capacity;
    let xml = Qcow2Volume {
        name,
        destination: dst,
        capacity,
    }
    .render()
    .context("Cannot render standalone volume XML")?;
    StorageVol::create_xml_from(&pool, &xml, &source, 0)
        .with_context(|| format!("Cannot copy volume {src} to {dst}"))?;
    ensure!(
        read_qcow2_metadata(Path::new(dst))?.backing_file.is_none(),
        "The copy {dst} of {src} still has a backing file"
    );
    tracing::info!(
        disk_creation_seconds = start.elapsed().as_secs_f64(),
        method = "flatten",
        "Standalone disk provisioned"
    );
    Ok(())
}
