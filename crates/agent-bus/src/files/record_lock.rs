//! The record's lock: taken without waiting, and naming the process that holds it. Behind the
//! `record` feature, with the record itself.

use std::fs;
use std::path::Path;

use anyhow::{Result, anyhow};
use nix::errno::Errno;
use nix::fcntl::{Flock, FlockArg};
use tracing::instrument;

use super::{FileLock, held, lock_file};

/// As `lock_exclusive`, without waiting: `None` when another holds the lock.
///
/// # Errors
///
/// Fails when the file cannot be created, or locking fails other than by being held.
#[instrument(level = "debug", skip_all, fields(path = %path.display()))]
pub fn try_lock_exclusive(path: &Path) -> Result<Option<FileLock>> {
    match Flock::lock(lock_file(path)?, FlockArg::LockExclusiveNonblock) {
        Ok(lock) => held(lock, path).map(Some),
        Err((_, Errno::EWOULDBLOCK)) => Ok(None),
        Err((_, errno)) => {
            Err(anyhow!(errno).context(format!("Unable to lock {}", path.display())))
        }
    }
}

/// The process that holds, or last held, the lock at `path`, as it wrote itself into the file;
/// `None` when the file is missing or names no process.
pub fn lock_holder(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::lock_exclusive;
    use super::super::tests::{mode, scratch};
    use crate::files::PRIVATE_FILE_MODE;

    #[test]
    fn a_held_lock_is_not_waited_for() {
        let dir = scratch("try-lock");
        let path = dir.join("record.lock");
        let held = try_lock_exclusive(&path).unwrap();
        assert!(held.is_some());
        assert_eq!(mode(&path), PRIVATE_FILE_MODE);
        assert!(try_lock_exclusive(&path).unwrap().is_none());
        drop(held);
        assert!(try_lock_exclusive(&path).unwrap().is_some());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_lock_names_its_holder() {
        let dir = scratch("holder");
        let path = dir.join("record.lock");
        // A stale or foreign file names nobody.
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, "not a pid").unwrap();
        assert_eq!(lock_holder(&path), None);
        let lock = try_lock_exclusive(&path).unwrap().unwrap();
        assert_eq!(lock_holder(&path), Some(std::process::id()));
        drop(lock);
        fs::write(&path, "").unwrap();
        let lock = lock_exclusive(&path).unwrap();
        assert_eq!(lock_holder(&path), Some(std::process::id()));
        drop(lock);
        assert_eq!(lock_holder(&dir.join("missing.lock")), None);
        fs::remove_dir_all(&dir).unwrap();
    }
}
