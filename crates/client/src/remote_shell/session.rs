//! Passing a remote shell through to the local terminal: typed bytes to the node (watching
//! for `~.`), the node's output to stdout, and window resizes, until the shell ends.

use std::io::{self, Read, Write};
use std::thread;

use anyhow::{Context, Result};
use jato::{Event, Exit, Session, ShellOptions};
use tokio::sync::mpsc::{self, Receiver, Sender};

use super::escape::EscapeFilter;
use super::terminal::{self, RawMode, ResizeWatcher};

/// Bytes read from the terminal at a time.
const KEY_BUFFER: usize = 4096;
/// Reads of typed bytes queued for the node.
const KEY_QUEUE: usize = 64;
/// The exit code when the shell's own is not known, as OpenSSH uses for its own errors.
const UNKNOWN_EXIT: i32 = 255;

/// Run an interactive shell on `session` in the local terminal until it ends or `~.` is
/// typed. Returns the exit code for `sherpa ssh` to exit with: the shell's, or 255.
#[tracing::instrument(skip_all, fields(node = %session.node()))]
pub async fn run_shell(session: &Session) -> Result<i32> {
    let (columns, rows) = terminal::size();
    let mut options = ShellOptions::default();
    options.term = terminal::term_name();
    options.columns = columns;
    options.rows = rows;
    let mut shell = session
        .shell(&options)
        .await
        .context("Failed to open a shell")?;

    let raw_mode = RawMode::enable()?;
    let mut keys = read_keys();
    let mut resizes = ResizeWatcher::new()?;
    let mut escape = EscapeFilter::new();
    let mut stdout = io::stdout();
    let mut exit = None;
    loop {
        tokio::select! {
            event = shell.next_event() => match event.context("Failed to read from the shell")? {
                Some(Event::Stdout(bytes) | Event::Stderr(bytes)) => {
                    stdout.write_all(&bytes).context("Failed to write to the terminal")?;
                    stdout.flush().context("Failed to write to the terminal")?;
                }
                Some(Event::Exit(status)) => exit = Some(status),
                Some(other) => tracing::debug!(?other, "Shell event not shown"),
                None => break,
            },
            Some(typed) = keys.recv() => {
                let filtered = escape.filter(&typed);
                if !filtered.send.is_empty() {
                    shell
                        .send_bytes(&filtered.send)
                        .await
                        .context("Failed to send keys to the shell")?;
                }
                if filtered.disconnect {
                    tracing::debug!("Disconnected with the ~. escape");
                    break;
                }
            }
            Some((columns, rows)) = resizes.changed() => {
                shell
                    .resize(columns, rows)
                    .await
                    .context("Failed to resize the remote terminal")?;
            }
        }
    }
    drop(raw_mode);
    Ok(exit_code(exit.as_ref()))
}

/// Read typed bytes on a thread of their own, as reading the terminal blocks. The thread
/// ends when the terminal closes or the receiver is dropped; a read still waiting when the
/// shell ends is left behind and ends with the process.
fn read_keys() -> Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel(KEY_QUEUE);
    thread::spawn(move || forward_keys(&sender));
    receiver
}

fn forward_keys(sender: &Sender<Vec<u8>>) {
    let mut stdin = io::stdin();
    let mut buffer = [0; KEY_BUFFER];
    loop {
        match stdin.read(&mut buffer) {
            Ok(0) => {
                tracing::debug!("Terminal input closed");
                return;
            }
            Ok(read) => {
                if sender.blocking_send(buffer[..read].to_vec()).is_err() {
                    return;
                }
            }
            Err(error) => {
                tracing::debug!(%error, "Failed to read the terminal");
                return;
            }
        }
    }
}

/// The exit code for how the shell ended: its own if it fits in a byte, otherwise 255.
fn exit_code(exit: Option<&Exit>) -> i32 {
    match exit {
        Some(Exit::Code(code)) => match u8::try_from(*code) {
            Ok(code) => i32::from(code),
            Err(_) => UNKNOWN_EXIT,
        },
        Some(other) => {
            tracing::debug!(?other, "Shell ended without an exit code");
            UNKNOWN_EXIT
        }
        None => UNKNOWN_EXIT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_remote_exit_code_is_kept() {
        assert_eq!(exit_code(Some(&Exit::Code(0))), 0);
        assert_eq!(exit_code(Some(&Exit::Code(3))), 3);
    }

    #[test]
    fn codes_beyond_a_byte_and_unknown_endings_are_255() {
        assert_eq!(exit_code(Some(&Exit::Code(300))), 255);
        assert_eq!(exit_code(None), 255);
    }
}
