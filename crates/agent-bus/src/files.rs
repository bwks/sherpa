//! The state directory, `$XDG_STATE_HOME/agent-bus/`, and the JSON Lines files kept in it.
//!
//! The directory is private to the user (0700) on the host and in the guest alike; every command
//! that writes into it makes it so with `private_dir`. The JSON Lines files (`results.jsonl` in
//! the guest, `listen-<agent>.jsonl` on the host) are 0600, one object per line with an `at_ms`
//! field, and are pruned to the stream's retention: on start, and at most once a day before an
//! append.
//!
//! Files that more than one process changes (the host's registry, the guest's generation ledger)
//! are changed under an exclusive lock on a file beside them (`lock_exclusive`).

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use nix::fcntl::{Flock, FlockArg};
use serde::Deserialize;
use tracing::instrument;

use crate::channel::RETENTION;

/// The directory under `$XDG_STATE_HOME`.
const STATE_SUBDIR: &str = "agent-bus";
/// The mode of the state directory: private to the user.
pub const DIR_MODE: u32 = 0o700;
/// The mode of a private file in it.
pub const PRIVATE_FILE_MODE: u32 = 0o600;
/// How often a JSON Lines file is pruned while it is appended to.
const PRUNE_EVERY: Duration = Duration::from_hours(24);

/// The one field of a line that pruning reads.
#[derive(Deserialize)]
struct Stamp {
    at_ms: Option<u64>,
}

/// A held exclusive lock on a file. Dropping it releases it.
#[derive(Debug)]
pub struct FileLock {
    _lock: Flock<File>,
}

/// A JSON Lines file in the state directory, pruned to the stream's retention.
#[derive(Debug, Clone)]
pub struct JsonLines {
    path: PathBuf,
    /// When it was last pruned; `None` before the first prune.
    pruned_at_ms: Option<u64>,
}

/// The state directory of this user, from the environment.
///
/// # Errors
///
/// Fails when neither `XDG_STATE_HOME` nor `HOME` is an absolute path.
pub fn standard_state_dir() -> Result<PathBuf> {
    state_dir(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
}

/// The directory for the bus's state: `$XDG_STATE_HOME/agent-bus`, or
/// `~/.local/state/agent-bus`. A relative `XDG_STATE_HOME` is ignored, as the spec says.
fn state_dir(xdg_state_home: Option<OsString>, home: Option<OsString>) -> Result<PathBuf> {
    let base = match xdg_state_home.map(PathBuf::from) {
        Some(path) if path.is_absolute() => path,
        _ => {
            let home = home
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .context("Neither XDG_STATE_HOME nor HOME is set to an absolute path")?;
            home.join(".local").join("state")
        }
    };
    Ok(base.join(STATE_SUBDIR))
}

/// Create `dir` and its parents if missing, and set it to 0700.
///
/// # Errors
///
/// Fails when the directory cannot be created or its mode set.
pub fn private_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("Unable to create {}", dir.display()))?;
    fs::set_permissions(dir, fs::Permissions::from_mode(DIR_MODE))
        .with_context(|| format!("Unable to set the mode of {}", dir.display()))
}

/// The time now, in milliseconds since the Unix epoch; 0 for a clock before it.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// `duration` in milliseconds, saturating.
pub fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Replace the file at `path` with `text`, atomically: a reader sees the old file or the new
/// one. The text goes to a temporary file next to it, with `mode` whatever the umask, flushed to
/// disk, then renamed over it; the directory is flushed after, so the rename lasts. The
/// directory must exist. State files use `PRIVATE_FILE_MODE`.
///
/// # Errors
///
/// Fails when the temporary file cannot be written or renamed over the file.
#[instrument(level = "debug", skip_all, fields(path = %path.display(), mode = format_args!("{mode:o}")))]
pub fn write_atomic(path: &Path, text: &str, mode: u32) -> Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}.tmp", std::process::id()));
    let temporary = PathBuf::from(temporary);
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(&temporary)
        .with_context(|| format!("Unable to open {}", temporary.display()))?;
    // `mode` is masked by the umask and ignored for a file that exists.
    fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))
        .with_context(|| format!("Unable to set the mode of {}", temporary.display()))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .with_context(|| format!("Unable to write {}", temporary.display()))?;
    fs::rename(&temporary, path).with_context(|| {
        let _ = fs::remove_file(&temporary);
        format!(
            "Unable to move {} to {}",
            temporary.display(),
            path.display()
        )
    })?;
    if let Some(dir) = path.parent()
        && let Err(error) = File::open(dir).and_then(|dir| dir.sync_all())
    {
        eprintln!("Unable to flush {}: {error}", dir.display());
    }
    Ok(())
}

impl JsonLines {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            pruned_at_ms: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append `line` and its newline, after pruning the file if it was not pruned in the last
    /// day. The line is written with one `write` to the file, with no buffer in between, so it
    /// is in the file when this returns. A failed prune is reported and the append goes on.
    ///
    /// # Errors
    ///
    /// Fails when the directory or the file cannot be written.
    #[instrument(level = "debug", skip_all, fields(path = %self.path.display()))]
    pub fn append(&mut self, line: &str, now_ms: u64) -> Result<()> {
        if self
            .pruned_at_ms
            .is_none_or(|at| now_ms.saturating_sub(at) >= millis(PRUNE_EVERY))
            && let Err(error) = self.prune(now_ms)
        {
            eprintln!("{error:#}");
        }
        if let Some(dir) = self.path.parent() {
            private_dir(dir)?;
        }
        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .mode(PRIVATE_FILE_MODE)
            .open(&self.path)
            .with_context(|| format!("Unable to open {}", self.path.display()))?;
        // `mode` is masked by the umask and ignored for a file that exists.
        fs::set_permissions(&self.path, fs::Permissions::from_mode(PRIVATE_FILE_MODE))
            .with_context(|| format!("Unable to set the mode of {}", self.path.display()))?;
        let mut text = String::with_capacity(line.len() + 1);
        text.push_str(line);
        text.push('\n');
        file.write_all(text.as_bytes())
            .with_context(|| format!("Unable to write {}", self.path.display()))
    }

    /// Drop the lines whose `at_ms` is older than the stream's retention. A line without one (a
    /// line cut short by a crash, say) is kept. The file is rewritten only when a line is
    /// dropped: to a temporary file next to it, mode 0600, flushed to disk, then renamed over
    /// it. Returns how many lines were dropped. A missing file has none.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or rewritten.
    #[instrument(level = "debug", skip_all, fields(path = %self.path.display()))]
    pub fn prune(&mut self, now_ms: u64) -> Result<usize> {
        self.pruned_at_ms = Some(now_ms);
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Unable to read {} to prune it", self.path.display())
                });
            }
        };
        let cutoff = now_ms.saturating_sub(millis(RETENTION));
        let mut kept = String::with_capacity(text.len());
        let mut dropped = 0;
        for line in text.split_inclusive('\n') {
            if stamp(line).is_some_and(|at| at < cutoff) {
                dropped += 1;
            } else {
                kept.push_str(line);
            }
        }
        if dropped == 0 {
            return Ok(0);
        }
        write_atomic(&self.path, &kept, PRIVATE_FILE_MODE)
            .with_context(|| format!("Unable to prune {}", self.path.display()))?;
        Ok(dropped)
    }
}

/// The `at_ms` of a line, if it is a JSON object that has one.
pub fn stamp(line: &str) -> Option<u64> {
    serde_json::from_str::<Stamp>(line).ok()?.at_ms
}

/// Take the exclusive lock on the file at `path`, created 0600 if missing (its directory too,
/// 0700), waiting for another holder to finish. The file stays: removing it would let two
/// processes lock two files of one name.
///
/// # Errors
///
/// Fails when the file cannot be created or locked.
#[instrument(level = "debug", skip_all, fields(path = %path.display()))]
pub fn lock_exclusive(path: &Path) -> Result<FileLock> {
    if let Some(dir) = path.parent() {
        private_dir(dir)?;
    }
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(PRIVATE_FILE_MODE)
        .open(path)
        .with_context(|| format!("Unable to open {}", path.display()))?;
    let lock = Flock::lock(file, FlockArg::LockExclusive).map_err(|(_, errno)| {
        anyhow!(errno).context(format!("Unable to lock {}", path.display()))
    })?;
    Ok(FileLock { _lock: lock })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000_000;
    const DAY: u64 = 24 * 60 * 60 * 1000;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agent-bus-files-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn line(at_ms: u64, text: &str) -> String {
        format!(r#"{{"at_ms":{at_ms},"text":"{text}"}}"#)
    }

    #[test]
    fn the_state_dir_follows_xdg_and_falls_back_to_home() {
        let dir = |xdg: Option<&str>, home: Option<&str>| {
            state_dir(xdg.map(OsString::from), home.map(OsString::from))
        };
        assert_eq!(
            dir(Some("/x/state"), Some("/home/u")).unwrap(),
            PathBuf::from("/x/state/agent-bus")
        );
        assert_eq!(
            dir(None, Some("/home/u")).unwrap(),
            PathBuf::from("/home/u/.local/state/agent-bus")
        );
        assert_eq!(
            dir(Some("relative"), Some("/home/u")).unwrap(),
            PathBuf::from("/home/u/.local/state/agent-bus")
        );
        assert!(dir(None, None).is_err());
    }

    #[test]
    fn a_private_dir_is_created_or_tightened_to_0700() {
        let root = scratch("dir");
        let dir = root.join("a").join("agent-bus");
        private_dir(&dir).unwrap();
        assert_eq!(mode(&dir), DIR_MODE);
        // An existing directory that is too open, as the host used to make it, is tightened.
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        private_dir(&dir).unwrap();
        assert_eq!(mode(&dir), DIR_MODE);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn pruning_drops_old_lines_keeps_new_ones_and_the_mode() {
        let root = scratch("prune");
        let mut lines = JsonLines::new(root.join("state").join("x.jsonl"));
        // Nothing to prune before the file exists.
        assert_eq!(lines.prune(NOW).unwrap(), 0);
        lines.append(&line(NOW - 8 * DAY, "old"), NOW).unwrap();
        lines.append(&line(NOW - 7 * DAY + 1, "edge"), NOW).unwrap();
        lines.append("not json", NOW).unwrap();
        lines.append(&line(NOW - DAY, "new"), NOW).unwrap();
        assert_eq!(mode(lines.path()), PRIVATE_FILE_MODE);
        // A looser mode is not carried over by the rewrite.
        fs::set_permissions(lines.path(), fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(lines.prune(NOW).unwrap(), 1);
        let text = fs::read_to_string(lines.path()).unwrap();
        assert_eq!(
            text,
            format!(
                "{}\nnot json\n{}\n",
                line(NOW - 7 * DAY + 1, "edge"),
                line(NOW - DAY, "new")
            )
        );
        assert_eq!(mode(lines.path()), PRIVATE_FILE_MODE);
        // No temporary file is left, and a second prune changes nothing.
        assert_eq!(fs::read_dir(root.join("state")).unwrap().count(), 1);
        assert_eq!(lines.prune(NOW).unwrap(), 0);
        assert_eq!(fs::read_to_string(lines.path()).unwrap(), text);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_append_prunes_first_on_the_first_append_and_then_once_a_day() {
        let root = scratch("daily");
        let path = root.join("x.jsonl");
        fs::create_dir_all(&root).unwrap();
        fs::write(&path, format!("{}\n", line(NOW - 8 * DAY, "old"))).unwrap();

        let mut lines = JsonLines::new(path.clone());
        lines.append(&line(NOW, "a"), NOW).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("{}\n", line(NOW, "a"))
        );
        // Within the day an append does not prune, even with an old line in the file.
        fs::write(&path, format!("{}\n", line(NOW - 8 * DAY, "old"))).unwrap();
        lines.append(&line(NOW + 1, "b"), NOW + 1).unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("old"));
        // A day later it does.
        lines.append(&line(NOW + DAY, "c"), NOW + DAY).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("{}\n{}\n", line(NOW + 1, "b"), line(NOW + DAY, "c"))
        );
        assert_eq!(mode(&path), PRIVATE_FILE_MODE);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_lock_is_exclusive() {
        let dir = scratch("lock");
        let path = dir.join("enroll.lock");
        let held = lock_exclusive(&path).unwrap();
        let file = OpenOptions::new().write(true).open(&path).unwrap();
        assert!(Flock::lock(file, FlockArg::LockExclusiveNonblock).is_err());
        drop(held);
        let file = OpenOptions::new().write(true).open(&path).unwrap();
        assert!(Flock::lock(file, FlockArg::LockExclusiveNonblock).is_ok());
        fs::remove_dir_all(&dir).unwrap();
    }
}
