//! One guest agent per name: an exclusive `flock` on `guest-<name>.lock`, next to the state
//! file, taken before the state is loaded and held for the agent's lifetime. Two agents sharing
//! a state file would each write their own view over the other's.
//!
//! The kernel releases the lock when the holder exits, however it exits, so a lock file left
//! behind by a dead agent does not block the next one. The file holds the holder's PID, for the
//! error the next agent gives. It is never removed: removing it while another agent opens it
//! would let two agents lock two different files of the same name. The descriptor is opened
//! close-on-exec, so Claude Code and what it starts do not inherit the lock.

use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process;

use anyhow::{Context, Result, anyhow};
use nix::errno::Errno;
use nix::fcntl::{Flock, FlockArg};

use crate::files::{PRIVATE_FILE_MODE, private_dir};

/// The held lock. Dropping it releases it.
#[derive(Debug)]
pub(super) struct InstanceLock {
    _lock: Flock<File>,
    path: PathBuf,
}

impl InstanceLock {
    /// Take the lock at `path` without waiting, and write this process's PID in it.
    ///
    /// # Errors
    ///
    /// Fails when another process holds it (naming that process when its PID can be read), or
    /// when the file cannot be created, locked or written.
    pub(super) fn acquire(path: &Path) -> Result<Self> {
        let dir = path.parent().context("The lock file has no directory")?;
        private_dir(dir)?;
        // Not truncated on open: until it is locked, the PID in it is the holder's.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(PRIVATE_FILE_MODE)
            .open(path)
            .with_context(|| format!("Unable to open {}", path.display()))?;
        let mut lock = match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
            Ok(lock) => lock,
            Err((_, Errno::EWOULDBLOCK)) => {
                let holder = fs::read_to_string(path)
                    .ok()
                    .and_then(|text| text.trim().parse::<u32>().ok())
                    .map_or_else(|| "another process".to_owned(), |pid| format!("PID {pid}"));
                return Err(anyhow!(
                    "Another guest agent of this name is running ({holder} holds {}): stop it first",
                    path.display()
                ));
            }
            Err((_, errno)) => {
                return Err(anyhow!(errno).context(format!("Unable to lock {}", path.display())));
            }
        };
        lock.set_len(0)
            .and_then(|()| lock.seek(SeekFrom::Start(0)))
            .and_then(|_| writeln!(lock, "{}", process::id()))
            .and_then(|()| lock.sync_all())
            .with_context(|| format!("Unable to write {}", path.display()))?;
        Ok(Self {
            _lock: lock,
            path: path.to_owned(),
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::env;
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::Duration;

    fn dir(label: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("agent-bus-lock-{label}-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_second_lock_fails_and_the_lock_is_free_after_drop() {
        let dir = dir("second");
        let path = dir.join("agent-bus").join("guest-dev.lock");
        let lock = InstanceLock::acquire(&path).unwrap();
        assert_eq!(lock.path(), path);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("{}\n", process::id())
        );
        // flock is per open file, so a second open in this process conflicts too.
        let error = InstanceLock::acquire(&path).unwrap_err().to_string();
        assert!(error.contains(&format!("PID {}", process::id())), "{error}");
        assert!(error.contains("guest-dev.lock"), "{error}");
        drop(lock);
        let again = InstanceLock::acquire(&path).unwrap();
        drop(again);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[ignore = "starts a real process and kills it; run with -- --ignored"]
    fn a_lock_left_by_a_killed_process_does_not_block() {
        let dir = dir("stale");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("guest-dev.lock");
        let held = dir.join("held");
        // One process, the shell that becomes the sleep, holds the lock and writes its PID.
        let mut holder = Command::new("sh")
            .arg("-c")
            .arg(r#"exec 9>>"$1"; flock --nonblock 9 && echo $$ > "$1" && touch "$2" && exec sleep 300"#)
            .arg("sh")
            .arg(&path)
            .arg(&held)
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        for _ in 0..100 {
            if held.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert!(held.exists(), "the shell never took the lock");
        let error = InstanceLock::acquire(&path).unwrap_err().to_string();
        assert!(error.contains(&format!("PID {}", holder.id())), "{error}");
        // SIGKILL: no clean-up runs and the file stays, but the kernel drops the lock.
        holder.kill().unwrap();
        holder.wait().unwrap();
        let lock = InstanceLock::acquire(&path);
        assert!(path.exists());
        assert!(
            lock.is_ok(),
            "the lock of a killed process still blocks: {lock:?}"
        );
        drop(lock);
        fs::remove_dir_all(&dir).unwrap();
    }
}
