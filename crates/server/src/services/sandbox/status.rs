//! What a sandbox, or a Docker store VM, reports on its status channel, a virtio-serial port
//! whose host side is a socket QEMU binds: one line, `started`, `loaded` or `failed: REASON`.

use std::io::ErrorKind;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;
use tokio::time;
use tracing::instrument;

use crate::services::error::ServiceError;

/// How long the status socket has to accept a connection once its domain has started.
const CONNECT_WAIT: Duration = Duration::from_secs(10);
/// The pause between two attempts to connect to the status socket.
const CONNECT_POLL: Duration = Duration::from_millis(200);
/// The prefix of a failure's status line, before its reason.
const FAILED_PREFIX: &str = "failed: ";
/// The longest status line read, newline included.
const MAX_LINE_BYTES: usize = 4096;
/// The most a VM may send before its status.
const MAX_BYTES_BEFORE_STATUS: usize = 64 * 1024;
/// The most characters of a failure's reason kept.
const MAX_REASON_CHARS: usize = 512;
/// The most characters of a line that is no status logged.
const MAX_LOGGED_CHARS: usize = 80;
/// The bytes read from the socket at a time.
const READ_CHUNK_BYTES: usize = 1024;

/// A status a sandbox or a Docker store VM reports.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Status {
    Started,
    Loaded,
    Failed(String),
}

/// The status channel of a started VM, read up to its first status.
pub(crate) struct StatusWatch {
    name: String,
    stream: UnixStream,
    pending: Vec<u8>,
    received: usize,
    skipped: usize,
}

impl Status {
    /// The status of line `line`, its control characters dropped and its reason capped, or
    /// `None` when it is not one.
    fn parse(line: &str) -> Option<Self> {
        let line = printable(line);
        match line.trim_end() {
            "started" => Some(Self::Started),
            "loaded" => Some(Self::Loaded),
            other => other
                .strip_prefix(FAILED_PREFIX)
                .map(|reason| Self::Failed(capped(reason, MAX_REASON_CHARS))),
        }
    }

    /// Succeed when this is `expected`, fail with the reason the VM `name` gave otherwise.
    ///
    /// # Errors
    ///
    /// Fails with [`ServiceError::SandboxFailed`] when the VM reported a failure or another
    /// status.
    pub(super) fn expect(self, expected: &Self, name: &str) -> Result<(), ServiceError> {
        match self {
            status if status == *expected => Ok(()),
            Self::Failed(reason) => Err(ServiceError::SandboxFailed {
                name: name.to_owned(),
                reason,
            }),
            other => Err(ServiceError::SandboxFailed {
                name: name.to_owned(),
                reason: format!("it reported {other:?}, not {expected:?}"),
            }),
        }
    }
}

impl StatusWatch {
    /// Connect to the status socket `socket` of VM `name`, which its domain binds when it
    /// starts.
    ///
    /// # Errors
    ///
    /// Fails when the socket does not accept a connection within `CONNECT_WAIT`.
    #[instrument(level = "debug")]
    pub(super) async fn connect(socket: &str, name: &str) -> Result<Self> {
        let started = Instant::now();
        loop {
            match UnixStream::connect(socket).await {
                Ok(stream) => return Ok(Self::of(stream, name)),
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

    /// The watch of VM `name` on connected stream `stream`.
    fn of(stream: UnixStream, name: &str) -> Self {
        Self {
            name: name.to_owned(),
            stream,
            pending: Vec::new(),
            received: 0,
            skipped: 0,
        }
    }

    /// Wait for the first status the VM reports, skipping lines that are no status.
    ///
    /// # Errors
    ///
    /// Fails when the stream cannot be read, ends before a status, as when the VM stopped, or
    /// carries a line or bytes beyond the limits.
    pub(super) async fn next(&mut self) -> Result<Status> {
        let mut chunk = [0; READ_CHUNK_BYTES];
        loop {
            if let Some(status) = self.take()? {
                return Ok(status);
            }
            let read = self
                .stream
                .read(&mut chunk)
                .await
                .with_context(|| format!("Cannot read the status of VM {}", self.name))?;
            self.receive(&chunk[..read])?;
        }
    }

    /// Whether the sandbox has reported its container started, from what it has sent by now,
    /// without waiting.
    ///
    /// # Errors
    ///
    /// Fails when the VM reported a failure or another status, or as `next` does.
    pub(crate) fn started(&mut self) -> Result<bool> {
        let mut chunk = [0; READ_CHUNK_BYTES];
        loop {
            if let Some(status) = self.take()? {
                status.expect(&Status::Started, &self.name)?;
                return Ok(true);
            }
            match self.stream.try_read(&mut chunk) {
                Ok(read) => self.receive(&chunk[..read])?,
                Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(false),
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("Cannot read the status of VM {}", self.name));
                }
            }
        }
    }

    /// Wait until the sandbox reports its container started.
    ///
    /// # Errors
    ///
    /// Fails when the VM reported a failure or another status, or as `next` does.
    pub(crate) async fn wait_started(&mut self) -> Result<()> {
        let status = self.next().await?;
        Ok(status.expect(&Status::Started, &self.name)?)
    }

    /// Add `bytes` read from the stream; none means the stream ended.
    fn receive(&mut self, bytes: &[u8]) -> Result<(), ServiceError> {
        if bytes.is_empty() {
            return Err(ServiceError::SandboxStatusClosed {
                name: self.name.clone(),
            });
        }
        self.received += bytes.len();
        self.pending.extend_from_slice(bytes);
        Ok(())
    }

    /// The first status among the complete lines received so far.
    ///
    /// # Errors
    ///
    /// Fails when a line is longer than `MAX_LINE_BYTES`, or more than
    /// `MAX_BYTES_BEFORE_STATUS` came without a status.
    fn take(&mut self) -> Result<Option<Status>, ServiceError> {
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            if end >= MAX_LINE_BYTES {
                return Err(self.too_long());
            }
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            if let Some(status) = Status::parse(&line) {
                tracing::info!(vm = %self.name, ?status, "VM reported its status");
                return Ok(Some(status));
            }
            if self.skipped == 0 {
                tracing::warn!(
                    vm = %self.name,
                    line = %capped(&printable(&line), MAX_LOGGED_CHARS),
                    "VM sent a line that is no status; later ones are not logged"
                );
            }
            self.skipped += 1;
        }
        if self.pending.len() >= MAX_LINE_BYTES {
            return Err(self.too_long());
        }
        if self.received > MAX_BYTES_BEFORE_STATUS {
            return Err(ServiceError::SandboxStatusFlood {
                name: self.name.clone(),
                limit: MAX_BYTES_BEFORE_STATUS,
            });
        }
        Ok(None)
    }

    /// The error of a status line longer than `MAX_LINE_BYTES`.
    fn too_long(&self) -> ServiceError {
        ServiceError::SandboxStatusTooLong {
            name: self.name.clone(),
            limit: MAX_LINE_BYTES,
        }
    }
}

/// `text` without its control characters.
fn printable(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// The first `max` characters of `text`, with an ellipsis when it was longer.
fn capped(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}...", &text[..end]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncWriteExt;

    use super::*;

    /// A watch on one end of a socket pair, and the guest's end.
    fn pair() -> (StatusWatch, UnixStream) {
        let (host, guest) = UnixStream::pair().unwrap();
        (StatusWatch::of(host, "srl1-abc12345"), guest)
    }

    #[test]
    fn sandbox_053_a_failure_the_vm_reports_fails_it_with_the_reason() {
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
            ServiceError::SandboxFailed { reason, .. } if reason == "cannot load the image: short read"
        ));
        assert!(Status::Loaded.expect(&Status::Started, "c1-lab").is_err());
    }

    #[tokio::test]
    async fn sandbox_060_a_status_line_longer_than_the_limit_fails_the_vm_before_it_is_read_whole()
    {
        let (mut watch, mut guest) = pair();
        let writer = tokio::spawn(async move {
            let flood = vec![b'x'; 1024 * 1024];
            let _ = guest.write_all(&flood).await;
        });
        let error = watch.next().await.unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<ServiceError>(),
                Some(ServiceError::SandboxStatusTooLong { limit: 4096, .. })
            ),
            "{error:#}"
        );
        assert!(watch.received < 2 * MAX_LINE_BYTES);
        drop(watch);
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn sandbox_061_lines_that_are_no_status_beyond_the_limit_fail_the_vm() {
        let (mut watch, mut guest) = pair();
        let writer = tokio::spawn(async move {
            let lines = b"junk\n".repeat(100_000);
            let _ = guest.write_all(&lines).await;
        });
        let error = watch.next().await.unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<ServiceError>(),
                Some(ServiceError::SandboxStatusFlood { .. })
            ),
            "{error:#}"
        );
        drop(watch);
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn sandbox_062_a_failure_reason_has_no_control_characters_and_is_capped() {
        let (mut watch, mut guest) = pair();
        let reason = format!("bad\x1b[31m\x07 {}", "y".repeat(2000));
        guest
            .write_all(format!("junk\nfailed: {reason}\n").as_bytes())
            .await
            .unwrap();
        let Status::Failed(read) = watch.next().await.unwrap() else {
            panic!("not a failure");
        };
        assert!(read.starts_with("bad[31m yyy"), "{read}");
        assert!(!read.chars().any(char::is_control));
        assert_eq!(read.chars().count(), 512 + 3);
        assert!(read.ends_with("y..."));
    }

    #[tokio::test]
    async fn sandbox_064_up_reads_a_sandboxes_status_as_it_polls_with_no_task_of_its_own() {
        let (mut watch, mut guest) = pair();
        assert!(!watch.started().unwrap());
        guest.write_all(b"started\n").await.unwrap();
        let mut seen = false;
        for _ in 0..100 {
            if watch.started().unwrap() {
                seen = true;
                break;
            }
            time::sleep(Duration::from_millis(10)).await;
        }
        assert!(seen);
        let (mut closed, guest) = pair();
        drop(guest);
        time::sleep(Duration::from_millis(10)).await;
        let error = closed.started().unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<ServiceError>(),
                Some(ServiceError::SandboxStatusClosed { .. })
            ),
            "{error:#}"
        );
    }
}
