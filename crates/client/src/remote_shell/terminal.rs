//! The local terminal: raw mode while the remote shell runs, its size and type, and its
//! resizes. On Windows the console also passes keys and output as VT sequences, as Unix
//! terminals do, so every platform reads typed bytes the same way.

use std::env;
#[cfg(windows)]
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::terminal;
#[cfg(unix)]
use tokio::signal::unix::{Signal, SignalKind, signal};
#[cfg(windows)]
use tokio::time::{Interval, MissedTickBehavior, interval};

#[cfg(windows)]
use crate::windows_console::{self, ConsoleModes};

/// The terminal type sent when `TERM` is not set, as on Windows.
const DEFAULT_TERM: &str = "xterm-256color";
/// The size sent when the terminal's size cannot be read.
const DEFAULT_SIZE: (u32, u32) = (80, 24);
/// How often Windows, which has no resize signal, checks the console's size.
#[cfg(windows)]
const RESIZE_POLL: Duration = Duration::from_millis(250);

/// Raw mode on the local terminal, restored when dropped, so every way out of the remote
/// shell (exit, `~.`, an error) leaves the terminal as it was.
pub(super) struct RawMode {
    #[cfg(windows)]
    console: ConsoleModes,
}

/// Reports changes to the local terminal's size.
pub(super) struct ResizeWatcher {
    last: (u32, u32),
    #[cfg(unix)]
    signal: Signal,
    #[cfg(windows)]
    interval: Interval,
}

impl RawMode {
    #[cfg(unix)]
    pub(super) fn enable() -> Result<Self> {
        terminal::enable_raw_mode().context("Failed to put the terminal in raw mode")?;
        Ok(Self {})
    }

    #[cfg(windows)]
    pub(super) fn enable() -> Result<Self> {
        terminal::enable_raw_mode().context("Failed to put the terminal in raw mode")?;
        match windows_console::enable_virtual_terminal() {
            Ok(console) => Ok(Self { console }),
            Err(error) => {
                if let Err(restore) = terminal::disable_raw_mode() {
                    tracing::warn!(error = %restore, "Failed to restore the terminal");
                }
                Err(error).context("Failed to turn on VT input and output in the console")
            }
        }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Err(error) = windows_console::restore(&self.console) {
            tracing::warn!(%error, "Failed to restore the console modes");
        }
        if let Err(error) = terminal::disable_raw_mode() {
            tracing::warn!(%error, "Failed to restore the terminal");
        }
    }
}

impl ResizeWatcher {
    #[cfg(unix)]
    pub(super) fn new() -> Result<Self> {
        Ok(Self {
            last: size(),
            signal: signal(SignalKind::window_change())
                .context("Failed to watch for terminal resizes")?,
        })
    }

    #[cfg(windows)]
    pub(super) fn new() -> Result<Self> {
        let mut interval = interval(RESIZE_POLL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        Ok(Self {
            last: size(),
            interval,
        })
    }

    /// Wait until the terminal's size changes, and return the new columns and rows.
    pub(super) async fn changed(&mut self) -> Option<(u32, u32)> {
        loop {
            self.wait().await?;
            let current = size();
            if current != self.last {
                self.last = current;
                return Some(current);
            }
        }
    }

    #[cfg(unix)]
    async fn wait(&mut self) -> Option<()> {
        self.signal.recv().await
    }

    #[cfg(windows)]
    async fn wait(&mut self) -> Option<()> {
        self.interval.tick().await;
        Some(())
    }
}

/// The local terminal type, from `TERM`.
pub(super) fn term_name() -> String {
    match env::var("TERM") {
        Ok(term) if !term.is_empty() => term,
        _ => DEFAULT_TERM.to_string(),
    }
}

/// The local terminal's columns and rows.
pub(super) fn size() -> (u32, u32) {
    match terminal::size() {
        Ok((columns, rows)) => (u32::from(columns), u32::from(rows)),
        Err(error) => {
            tracing::debug!(%error, "Terminal size unknown, using the default");
            DEFAULT_SIZE
        }
    }
}
