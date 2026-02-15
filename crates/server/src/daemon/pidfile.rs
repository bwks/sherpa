use anyhow::{bail, Context, Result};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::fs;
use std::path::Path;

use shared::konst::{SHERPAD_PID_FILE, SHERPA_BASE_DIR, SHERPA_LOG_DIR, SHERPA_RUN_DIR};

/// Ensure the run directory exists
pub fn ensure_run_dir() -> Result<()> {
    if !Path::new(&format!("{SHERPA_BASE_DIR}/{SHERPA_RUN_DIR}")).exists() {
        fs::create_dir_all(&format!("{SHERPA_BASE_DIR}/{SHERPA_RUN_DIR}")).context(format!(
            "Failed to create run directory: {}",
            SHERPA_RUN_DIR
        ))?;
    }
    Ok(())
}

/// Ensure the log directory exists
pub fn ensure_log_dir() -> Result<()> {
    if !Path::new(&format!("{SHERPA_BASE_DIR}/{SHERPA_LOG_DIR}")).exists() {
        fs::create_dir_all(&format!("{SHERPA_BASE_DIR}/{SHERPA_LOG_DIR}")).context(format!(
            "Failed to create log directory: {}",
            SHERPA_LOG_DIR
        ))?;
    }
    Ok(())
}

/// Write the current process PID to a file
pub fn write_pid(path: &str) -> Result<()> {
    let pid = std::process::id();
    fs::write(path, pid.to_string()).context(format!("Failed to write PID file: {}", path))?;
    Ok(())
}

/// Read PID from a file
pub fn read_pid(path: &str) -> Result<Option<u32>> {
    if !Path::new(path).exists() {
        return Ok(None);
    }

    let contents =
        fs::read_to_string(path).context(format!("Failed to read PID file: {}", path))?;

    let pid = contents
        .trim()
        .parse::<u32>()
        .context(format!("Invalid PID in file: {}", path))?;

    Ok(Some(pid))
}

/// Remove the PID file
pub fn remove_pid(path: &str) -> Result<()> {
    if Path::new(path).exists() {
        fs::remove_file(path).context(format!("Failed to remove PID file: {}", path))?;
    }
    Ok(())
}

/// Check if a process with the given PID is running
pub fn is_process_running(pid: u32) -> bool {
    // Send signal 0 to check if process exists without actually sending a signal
    let pid = Pid::from_raw(pid as i32);
    kill(pid, None).is_ok()
}

/// Check for stale PID file and clean it up if process is not running
#[allow(dead_code)]
pub fn check_stale_pidfile(path: &str) -> Result<bool> {
    match read_pid(path)? {
        Some(pid) => {
            if !is_process_running(pid) {
                remove_pid(path)?;
                Ok(true) // Was stale
            } else {
                Ok(false) // Not stale, process is running
            }
        }
        None => Ok(false), // No PID file
    }
}

/// Verify server is not already running, cleaning up stale PID files if necessary
pub fn verify_not_running() -> Result<()> {
    if let Some(pid) = read_pid(&format!(
        "{SHERPA_BASE_DIR}/{SHERPA_RUN_DIR}/{SHERPAD_PID_FILE}"
    ))? {
        if is_process_running(pid) {
            bail!("Server is already running (PID: {})", pid);
        } else {
            // Stale PID file - clean it up
            tracing::warn!("Found stale PID file, cleaning up");
            remove_pid(&format!(
                "{SHERPA_BASE_DIR}/{SHERPA_RUN_DIR}/{SHERPAD_PID_FILE}"
            ))?;
        }
    }
    Ok(())
}

/// Send a signal to the process in the PID file
pub fn send_signal(pid: u32, signal: Signal) -> Result<()> {
    let pid = Pid::from_raw(pid as i32);
    kill(pid, signal).context(format!("Failed to send signal {:?} to PID {}", signal, pid))?;
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
        assert!(read_pid_value.is_some());
        assert_eq!(read_pid_value.unwrap(), std::process::id());
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
        let current_pid = std::process::id();
        assert!(is_process_running(current_pid));

        // PID 99999 should not exist
        assert!(!is_process_running(99999));
    }

    #[test]
    fn test_check_stale_pidfile() {
        let temp_dir = TempDir::new().unwrap();
        let pid_file = temp_dir.path().join("test.pid");
        let pid_path = pid_file.to_str().unwrap();

        // Write a fake PID that doesn't exist
        fs::write(pid_path, "99999").unwrap();

        // Should detect and remove stale file
        let was_stale = check_stale_pidfile(pid_path).unwrap();
        assert!(was_stale);
        assert!(!Path::new(pid_path).exists());

        // Write current PID (which is running)
        write_pid(pid_path).unwrap();

        // Should not be stale
        let was_stale = check_stale_pidfile(pid_path).unwrap();
        assert!(!was_stale);
        assert!(Path::new(pid_path).exists());
    }
}
