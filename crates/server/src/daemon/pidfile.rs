use anyhow::{Context, Result, bail};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use std::fs;
use std::path::Path;

use shared::konst::{SHERPA_LOG_PATH, SHERPA_RUN_PATH, SHERPAD_PID_FILE_PATH};

/// The smallest PID a pidfile may hold. `kill` reads 0 as our own process group and negative
/// values as other groups, and 1 is init.
const MIN_DAEMON_PID: i32 = 2;

/// Ensure the run directory exists
pub fn ensure_run_dir() -> Result<()> {
    if !Path::new(SHERPA_RUN_PATH).exists() {
        fs::create_dir_all(SHERPA_RUN_PATH)
            .context(format!("Failed to create run directory: {SHERPA_RUN_PATH}"))?;
    }
    Ok(())
}

/// Ensure the log directory exists
pub fn ensure_log_dir() -> Result<()> {
    if !Path::new(SHERPA_LOG_PATH).exists() {
        fs::create_dir_all(SHERPA_LOG_PATH)
            .context(format!("Failed to create log directory: {SHERPA_LOG_PATH}"))?;
    }
    Ok(())
}

/// Write the current process PID to a file
pub fn write_pid(path: &str) -> Result<()> {
    let pid = std::process::id();
    fs::write(path, pid.to_string()).context(format!("Failed to write PID file: {path}"))?;
    Ok(())
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

/// Verify server is not already running, cleaning up stale PID files if necessary
pub fn verify_not_running() -> Result<()> {
    if let Some(pid) = read_pid(SHERPAD_PID_FILE_PATH)? {
        if is_process_running(pid) {
            bail!("Server is already running (PID: {pid})");
        }
        // Stale PID file - clean it up
        tracing::warn!("Found stale PID file, cleaning up");
        remove_pid(SHERPAD_PID_FILE_PATH)?;
    }
    Ok(())
}

/// Send a signal to the process in the PID file
pub fn send_signal(pid: Pid, signal: Signal) -> Result<()> {
    kill(pid, signal).context(format!("Failed to send signal {signal:?} to PID {pid}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_write_and_read_pid() {
        let temp_dir = TempDir::new().unwrap();
        let pid_file = temp_dir.path().join("test.pid");
        let pid_path = pid_file.to_str().unwrap();

        // Write current PID
        write_pid(pid_path).unwrap();

        // Read it back
        let read_pid_value = read_pid(pid_path).unwrap();
        assert_eq!(read_pid_value, Some(Pid::this()));
    }

    #[test]
    fn a_pid_that_would_signal_a_group_or_init_is_refused() {
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
    fn test_read_nonexistent_pid() {
        let result = read_pid("/tmp/nonexistent_pid_file_12345.pid").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_remove_pid() {
        let temp_dir = TempDir::new().unwrap();
        let pid_file = temp_dir.path().join("test.pid");
        let pid_path = pid_file.to_str().unwrap();

        // Write PID
        write_pid(pid_path).unwrap();
        assert!(Path::new(pid_path).exists());

        // Remove it
        remove_pid(pid_path).unwrap();
        assert!(!Path::new(pid_path).exists());

        // Removing again should not error
        remove_pid(pid_path).unwrap();
    }

    #[test]
    fn test_is_process_running() {
        // Current process should be running
        assert!(is_process_running(Pid::this()));

        // PID 99999 should not exist
        assert!(!is_process_running(Pid::from_raw(99999)));
    }
}
