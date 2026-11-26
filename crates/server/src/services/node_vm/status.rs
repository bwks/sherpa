//! What a node VM, or a Docker store VM, reports on its status channel: a virtio-serial port
//! whose host side is a socket QEMU binds in the lab's console folder. The guest writes one
//! line, `started`, `loaded` or `failed: REASON`, and sherpad reads it. A guest's write waits
//! until sherpad has connected.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use libvirt::Qemu;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixStream;
use tokio::{task, time};
use tracing::instrument;

use crate::services::error::ServiceError;

/// How long the status socket has to accept a connection once its domain has started.
const CONNECT_WAIT: Duration = Duration::from_secs(10);
/// The pause between two attempts to connect to the status socket.
const CONNECT_POLL: Duration = Duration::from_millis(200);
/// The prefix of a failure's status line, before its reason.
const FAILED_PREFIX: &str = "failed: ";

/// A status a node VM or a Docker store VM reports.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Status {
    Started,
    Loaded,
    Failed(String),
}

impl Status {
    /// The status of line `line`, or `None` when it is not one.
    fn parse(line: &str) -> Option<Self> {
        match line.trim_end() {
            "started" => Some(Self::Started),
            "loaded" => Some(Self::Loaded),
            other => other
                .strip_prefix(FAILED_PREFIX)
                .map(|reason| Self::Failed(reason.to_owned())),
        }
    }

    /// Succeed when this is `expected`, fail with the reason the VM `name` gave otherwise.
    ///
    /// # Errors
    ///
    /// Fails with [`ServiceError::NodeVmFailed`] when the VM reported a failure or another
    /// status.
    pub(super) fn expect(self, expected: &Self, name: &str) -> Result<(), ServiceError> {
        match self {
            status if status == *expected => Ok(()),
            Self::Failed(reason) => Err(ServiceError::NodeVmFailed {
                name: name.to_owned(),
                reason,
            }),
            other => Err(ServiceError::NodeVmFailed {
                name: name.to_owned(),
                reason: format!("it reported {other:?}, not {expected:?}"),
            }),
        }
    }
}

/// Connect to the status socket `socket` of VM `name`, which its domain binds when it starts.
///
/// # Errors
///
/// Fails when the socket does not accept a connection within `CONNECT_WAIT`.
#[instrument(level = "debug")]
pub(super) async fn connect(socket: &str, name: &str) -> Result<UnixStream> {
    let started = Instant::now();
    loop {
        match UnixStream::connect(socket).await {
            Ok(stream) => return Ok(stream),
            Err(error) if started.elapsed() < CONNECT_WAIT => {
                tracing::debug!(%socket, %error, "Status socket not accepting yet");
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Cannot connect to the status socket {socket} of VM {name}")
                });
            }
        }
        time::sleep(CONNECT_POLL).await;
    }
}

/// Read the first status VM `name` reports on `stream`, skipping lines that are no status.
///
/// # Errors
///
/// Fails when the stream cannot be read, or ends before a status, as when the VM stopped.
pub(super) async fn read_status(stream: UnixStream, name: &str) -> Result<Status> {
    let mut lines = BufReader::new(stream).lines();
    while let Some(line) = lines
        .next_line()
        .await
        .with_context(|| format!("Cannot read the status of VM {name}"))?
    {
        if let Some(status) = Status::parse(&line) {
            tracing::info!(vm = %name, ?status, "VM reported its status");
            return Ok(status);
        }
        tracing::warn!(vm = %name, %line, "VM sent a line that is no status");
    }
    Err(ServiceError::NodeVmStatusClosed {
        name: name.to_owned(),
    }
    .into())
}

/// Define domain `name` again with kernel command line `cmdline`, from its next boot on.
///
/// # Errors
///
/// Fails when libvirt cannot redefine the domain.
pub(super) async fn set_cmdline(qemu: &Arc<Qemu>, name: &str, cmdline: String) -> Result<()> {
    let qemu = Arc::clone(qemu);
    let name = name.to_owned();
    task::spawn_blocking(move || {
        let conn = qemu.connect()?;
        libvirt::set_node_vm_cmdline(&conn, &name, &cmdline)
    })
    .await
    .context("The node VM redefinition panicked")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_vm_053_a_failure_the_vm_reports_fails_it_with_the_reason() {
        assert_eq!(Status::parse("started\n"), Some(Status::Started));
        assert_eq!(Status::parse("loaded"), Some(Status::Loaded));
        assert_eq!(
            Status::parse("failed: cannot start the container: no such image"),
            Some(Status::Failed(
                "cannot start the container: no such image".to_owned()
            ))
        );
        assert_eq!(Status::parse("Welcome"), None);
        assert!(Status::Started.expect(&Status::Started, "c1-lab").is_ok());
        let failed = Status::Failed("cannot load the image: short read".to_owned())
            .expect(&Status::Loaded, "sherpa-flatcar-docker-0123")
            .unwrap_err();
        assert!(matches!(
            &failed,
            ServiceError::NodeVmFailed { reason, .. } if reason == "cannot load the image: short read"
        ));
        assert!(Status::Loaded.expect(&Status::Started, "c1-lab").is_err());
    }
}
