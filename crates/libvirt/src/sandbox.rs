//! Sandbox domains: waiting for a domain to stop.

use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tracing::instrument;
use virt::connect::Connect;
use virt::domain::Domain;

use crate::error::LibvirtError;

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
