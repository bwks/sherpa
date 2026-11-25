use anyhow::{Context, Result, anyhow, bail};
use nix::errno::Errno;
use nix::fcntl::{Flock, FlockArg};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use std::ffi::OsStr;
use std::fs::{self, DirBuilder, File, OpenOptions, Permissions};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::process;
use tracing::instrument;

use crate::services::error::ServiceError;

pub const SHERPAD_PID_FILE_PATH: &str = "/opt/sherpa/run/sherpad.pid";
pub(crate) const SHERPAD_LOG_FILE_PATH: &str = "/opt/sherpa/logs/sherpad.log";
const SHERPA_RUN_PATH: &str = "/opt/sherpa/run";
const SHERPA_LOG_PATH: &str = "/opt/sherpa/logs";
/// The log directory: `sherpa` and its group only.
const SHERPA_LOG_DIR_MODE: u32 = 0o750;
/// sherpad's log file, when it logs to a file: `sherpa` writes it, its group may read it.
const SHERPAD_LOG_FILE_MODE: u32 = 0o640;

/// The smallest PID a pidfile may hold. `kill` reads 0 as our own process group and negative
/// values as other groups, and 1 is init.
const MIN_DAEMON_PID: i32 = 2;
/// The program name a PID file's process must have been started as to count as the server.
const SHERPAD_PROGRAM_NAME: &str = "sherpad";

/// Ensure the run directory exists
pub fn ensure_run_dir() -> Result<()> {
    if !Path::new(SHERPA_RUN_PATH).exists() {
        fs::create_dir_all(SHERPA_RUN_PATH)
            .context(format!("Failed to create run directory: {SHERPA_RUN_PATH}"))?;
    }
    Ok(())
}

/// Ensure the log directory exists, created `sherpa` and its group only. An existing one keeps
/// its mode; the install and update scripts set it.
pub fn ensure_log_dir() -> Result<()> {
    if !Path::new(SHERPA_LOG_PATH).exists() {
        DirBuilder::new()
            .recursive(true)
            .mode(SHERPA_LOG_DIR_MODE)
            .create(SHERPA_LOG_PATH)
            .context(format!("Failed to create log directory: {SHERPA_LOG_PATH}"))?;
    }
    Ok(())
}

/// Open sherpad's log file at `path` for appending, creating it if needed. The file is given
/// `SHERPAD_LOG_FILE_MODE` whatever the umask, and a file left wider by an older release is
/// narrowed to it.
#[instrument(level = "debug", fields(path = %path.display()))]
pub fn open_log_file(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(SHERPAD_LOG_FILE_MODE)
        .open(path)
        .with_context(|| format!("Failed to open log file: {}", path.display()))?;
    file.set_permissions(Permissions::from_mode(SHERPAD_LOG_FILE_MODE))
        .with_context(|| format!("Failed to set permissions on {}", path.display()))?;
    Ok(file)
}

/// Take the exclusive lock on the PID file at `path` and write this process's PID to it. The lock
/// is held until the returned file is dropped, so a second server fails here.
///
/// # Errors
///
/// Fails when another process holds the lock, the file cannot be opened, locked or written, or
/// the file at `path` was replaced while it was being locked.
#[instrument(level = "debug")]
pub fn lock_pid_file(path: &str) -> Result<Flock<File>> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .with_context(|| format!("Failed to open PID file: {path}"))?;
    let mut lock = match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
        Ok(lock) => lock,
        Err((_, Errno::EWOULDBLOCK)) => match read_pid(path) {
            Ok(Some(pid)) => bail!(ServiceError::PidFileLocked {
                pid,
                path: path.to_owned(),
            }),
            _ => bail!("Server is already running: {path} is locked"),
        },
        Err((_, errno)) => {
            return Err(anyhow!(errno).context(format!("Failed to lock PID file: {path}")));
        }
    };
    let on_disk = fs::metadata(path).with_context(|| format!("Failed to read PID file: {path}"))?;
    let locked = lock
        .metadata()
        .with_context(|| format!("Failed to read PID file: {path}"))?;
    if (on_disk.dev(), on_disk.ino()) != (locked.dev(), locked.ino()) {
        bail!("PID file {path} was replaced while it was being locked; start sherpad again");
    }
    lock.set_len(0)
        .with_context(|| format!("Failed to empty PID file: {path}"))?;
    write!(lock, "{}", process::id())
        .with_context(|| format!("Failed to write PID file: {path}"))?;
    Ok(lock)
}

/// Read the PID from a pidfile. A PID that `kill` would read as a process group, or init, is an
/// error, so a damaged pidfile can never make `stop` signal the wrong processes.
pub fn read_pid(path: &str) -> Result<Option<Pid>> {
    if !Path::new(path).exists() {
        return Ok(None);
    }

    let contents = fs::read_to_string(path).context(format!("Failed to read PID file: {path}"))?;

    let pid = contents
        .trim()
        .parse::<i32>()
        .context(format!("Invalid PID in file: {path}"))?;
    if pid < MIN_DAEMON_PID {
        bail!("PID file {path} holds {pid}, which is not a daemon's PID");
    }

    Ok(Some(Pid::from_raw(pid)))
}

/// Remove the PID file
pub fn remove_pid(path: &str) -> Result<()> {
    if Path::new(path).exists() {
        fs::remove_file(path).context(format!("Failed to remove PID file: {path}"))?;
    }
    Ok(())
}

/// Check if a process with the given PID is running
pub fn is_process_running(pid: Pid) -> bool {
    // Send signal 0 to check if process exists without actually sending a signal
    kill(pid, None).is_ok()
}

/// Whether the process `pid` was started as `sherpad`, by the program name in its command line.
fn is_sherpad_process(pid: Pid) -> bool {
    let Ok(cmdline) = fs::read(format!("/proc/{pid}/cmdline")) else {
        return false;
    };
    let program = cmdline.split(|byte| *byte == 0).next().unwrap_or_default();
    Path::new(OsStr::from_bytes(program)).file_name() == Some(OsStr::new(SHERPAD_PROGRAM_NAME))
}

/// The PID of the server the PID file at `path` names, when that process runs and is `sherpad`.
/// A stale file, or one naming another program's process, reads as no server.
///
/// # Errors
///
/// Fails when the PID file cannot be read or holds no daemon's PID.
pub fn running_server_pid(path: &str) -> Result<Option<Pid>> {
    Ok(read_pid(path)?.filter(|pid| is_process_running(*pid) && is_sherpad_process(*pid)))
}

/// Verify no server is running, by the PID file.
pub fn verify_not_running() -> Result<()> {
    if let Some(pid) = running_server_pid(SHERPAD_PID_FILE_PATH)? {
        bail!("Server is already running (PID: {pid})");
    }
    Ok(())
}

/// Send a signal to the process in the PID file
pub fn send_signal(pid: Pid, signal: Signal) -> Result<()> {
    kill(pid, signal).context(format!("Failed to send signal {signal:?} to PID {pid}"))?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};
    use std::thread;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    pub(crate) fn spawn_sherpad_stand_in() -> Child {
        let child = Command::new("sh")
            .arg0(SHERPAD_PROGRAM_NAME)
            .args(["-c", "while :; do sleep 1; done"])
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while fs::read(format!("/proc/{}/cmdline", child.id()))
            .unwrap()
            .is_empty()
        {
            assert!(
                Instant::now() < deadline,
                "the stand-in has no command line"
            );
            thread::sleep(Duration::from_millis(10));
        }
        child
    }

    #[test]
    fn sherpad_daemon_003_a_pid_that_would_signal_a_group_or_init_is_refused() {
        let temp_dir = TempDir::new().unwrap();
        let pid_file = temp_dir.path().join("test.pid");
        let pid_path = pid_file.to_str().unwrap();
        for contents in ["0", "1", "-1", "-42", "4294967295", "abc", ""] {
            fs::write(pid_path, contents).unwrap();
            assert!(read_pid(pid_path).is_err(), "'{contents}' accepted");
        }
        fs::write(pid_path, "4242\n").unwrap();
        assert_eq!(read_pid(pid_path).unwrap(), Some(Pid::from_raw(4242)));
    }

    #[test]
    fn sherpad_daemon_002_the_pid_file_holds_the_pid_and_a_missing_one_reads_as_no_server() {
        let temp_dir = TempDir::new().unwrap();
        let pid_file = temp_dir.path().join("test.pid");
        let pid_path = pid_file.to_str().unwrap();

        assert!(read_pid(pid_path).unwrap().is_none());

        fs::write(pid_path, "999999999999").unwrap();
        let lock = lock_pid_file(pid_path).unwrap();
        assert_eq!(read_pid(pid_path).unwrap(), Some(Pid::this()));

        remove_pid(pid_path).unwrap();
        drop(lock);
        assert!(!Path::new(pid_path).exists());

        // Removing again should not error
        remove_pid(pid_path).unwrap();
    }

    #[test]
    fn sherpad_daemon_011_a_second_start_is_refused_while_the_first_holds_the_pid_file_lock() {
        let temp_dir = TempDir::new().unwrap();
        let pid_file = temp_dir.path().join("sherpad.pid");
        let pid_path = pid_file.to_str().unwrap();

        let first = lock_pid_file(pid_path).unwrap();
        let error = lock_pid_file(pid_path).expect_err("a second lock was taken");
        assert_eq!(
            error.downcast_ref::<ServiceError>(),
            Some(&ServiceError::PidFileLocked {
                pid: Pid::this(),
                path: pid_path.to_owned(),
            })
        );
        assert_eq!(read_pid(pid_path).unwrap(), Some(Pid::this()));

        drop(first);
        lock_pid_file(pid_path).unwrap();
    }

    #[test]
    fn server_file_modes_009_the_log_file_is_created_and_narrowed_to_its_mode() {
        let temp_dir = TempDir::new().unwrap();
        let new_log = temp_dir.path().join("new.log");
        open_log_file(&new_log).unwrap();
        assert_eq!(
            fs::metadata(&new_log).unwrap().permissions().mode() & 0o777,
            SHERPAD_LOG_FILE_MODE
        );

        let old_log = temp_dir.path().join("old.log");
        fs::write(&old_log, "kept\n").unwrap();
        fs::set_permissions(&old_log, Permissions::from_mode(0o644)).unwrap();
        open_log_file(&old_log).unwrap();
        assert_eq!(
            fs::metadata(&old_log).unwrap().permissions().mode() & 0o777,
            SHERPAD_LOG_FILE_MODE
        );
        assert_eq!(fs::read_to_string(&old_log).unwrap(), "kept\n");
    }

    #[test]
    fn sherpad_daemon_004_a_process_is_running_until_it_has_exited() {
        assert!(is_process_running(Pid::this()));

        let mut child = Command::new("sleep").arg("60").spawn().unwrap();
        let pid = Pid::from_raw(i32::try_from(child.id()).unwrap());
        assert!(is_process_running(pid));
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!is_process_running(pid));
    }
}
