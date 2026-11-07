use anyhow::{Context, Result, bail};
use nix::sys::signal::Signal;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

use super::pidfile::{
    ensure_log_dir, ensure_run_dir, is_process_running, read_pid, remove_pid, send_signal,
    verify_not_running, write_pid,
};
use super::server::run_server;
use shared::konst::{SHERPAD_LOG_FILE_PATH, SHERPAD_PID_FILE_PATH};

/// Start the sherpad daemon
pub async fn start_daemon(foreground: bool) -> Result<()> {
    // Ensure directories exist
    ensure_run_dir()?;
    ensure_log_dir()?;

    // Check if already running
    verify_not_running()?;

    if foreground {
        // Foreground mode: run server directly in current process
        tracing::info!("Starting sherpad in foreground mode");
        write_pid(SHERPAD_PID_FILE_PATH)?;

        // Run server and clean up PID file on exit
        let result = run_server(true).await;

        remove_pid(SHERPAD_PID_FILE_PATH)?;

        result
    } else {
        // Background mode: spawn as child process
        tracing::info!("Starting sherpad in background mode");

        // Spawn sherpad in a detached child process using nohup pattern
        let exe = std::env::current_exe()?;

        // We'll use a simple fork-like approach by spawning ourselves with a special flag
        // The child process will write its own PID
        // The child's stderr goes to the daemon log, so a failure before its logging starts,
        // such as a broken sherpa.toml, is still recorded.
        let stderr = OpenOptions::new()
            .create(true)
            .append(true)
            .open(SHERPAD_LOG_FILE_PATH)
            .with_context(|| format!("Failed to open log file: {SHERPAD_LOG_FILE_PATH}"))?;
        let mut child = Command::new(exe)
            .arg("--background-child")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr))
            .spawn()
            .context("Failed to spawn background process")?;

        // Give the child a moment to start and write its PID
        thread::sleep(Duration::from_millis(500));
        check_still_running(&mut child)?;

        tracing::info!(pid = child.id(), "sherpad started successfully");
        Ok(())
    }
}

/// Check a just-spawned daemon is still running.
///
/// `try_wait` reaps a child that already exited; `kill(pid, 0)` would still find it, as a zombie,
/// and report it running.
fn check_still_running(child: &mut Child) -> Result<()> {
    match child
        .try_wait()
        .context("Failed to check the background process")?
    {
        Some(status) => bail!(
            "Failed to start sherpad: it exited at once ({status}); see {SHERPAD_LOG_FILE_PATH}"
        ),
        None => Ok(()),
    }
}

/// Internal function to run as background child (called by start_daemon)
pub async fn run_background_child() -> Result<()> {
    // Write our own PID
    write_pid(SHERPAD_PID_FILE_PATH)?;

    // Run server and clean up on exit
    let result = run_server(false).await;

    remove_pid(SHERPAD_PID_FILE_PATH)?;

    result
}

/// Stop the sherpad daemon
pub fn stop_daemon(force: bool) -> Result<()> {
    // Read PID from file
    let pid = match read_pid(SHERPAD_PID_FILE_PATH)? {
        Some(pid) => pid,
        None => {
            bail!("Server is not running");
        }
    };

    // Check if process is actually running
    if !is_process_running(pid) {
        tracing::warn!(%pid, "Server is not running (stale PID file found)");
        remove_pid(SHERPAD_PID_FILE_PATH)?;
        return Ok(());
    }

    tracing::info!(%pid, "Stopping sherpad");

    // Send SIGTERM for graceful shutdown
    if let Err(e) = send_signal(pid, Signal::SIGTERM) {
        if force {
            tracing::warn!(error = %e, "Failed to send SIGTERM, trying SIGKILL");
        } else {
            bail!("Failed to stop server: {}", e);
        }
    } else {
        // Wait up to 10 seconds for process to exit
        let mut waited = 0;
        let wait_interval = 500; // ms
        let max_wait = 10000; // ms

        while waited < max_wait {
            thread::sleep(Duration::from_millis(wait_interval));
            waited += wait_interval;

            if !is_process_running(pid) {
                // Process has exited
                remove_pid(SHERPAD_PID_FILE_PATH)?;
                tracing::info!("sherpad stopped successfully");
                return Ok(());
            }
        }

        // Process still running after timeout
        if !force {
            bail!(
                "Server did not stop gracefully after {} seconds. Use --force to kill it.",
                max_wait / 1000
            );
        }
    }

    // Force kill if we get here
    tracing::warn!("Server did not stop gracefully, forcing shutdown with SIGKILL");
    send_signal(pid, Signal::SIGKILL).context("Failed to send SIGKILL")?;

    // Wait a bit for SIGKILL to take effect
    thread::sleep(Duration::from_millis(1000));

    if is_process_running(pid) {
        bail!("Failed to kill server process");
    }

    remove_pid(SHERPAD_PID_FILE_PATH)?;
    tracing::info!("sherpad stopped successfully");
    Ok(())
}

/// Restart the sherpad daemon
pub async fn restart_daemon(foreground: bool) -> Result<()> {
    tracing::info!("Restarting sherpad");

    // Stop the daemon if it's running
    if let Some(pid) = read_pid(SHERPAD_PID_FILE_PATH)? {
        if is_process_running(pid) {
            stop_daemon(false)?;
            // Give it a moment to fully stop
            thread::sleep(Duration::from_millis(2000));
        } else {
            // Clean up stale PID file
            tracing::debug!("Found stale PID file, cleaning up");
            remove_pid(SHERPAD_PID_FILE_PATH)?;
        }
    }

    // Start the daemon
    start_daemon(foreground).await
}

/// Show the status of the sherpad daemon
pub fn status_daemon() -> Result<()> {
    match read_pid(SHERPAD_PID_FILE_PATH)? {
        Some(pid) => {
            if is_process_running(pid) {
                println!("sherpad is running (pid: {pid})");
                Ok(())
            } else {
                eprintln!("sherpad is not running (stale PID file found)");
                std::process::exit(1);
            }
        }
        None => {
            eprintln!("sherpad is not running");
            std::process::exit(1);
        }
    }
}

/// Show the logs of the sherpad daemon
pub fn logs_daemon(follow: bool) -> Result<()> {
    let log_path = Path::new(SHERPAD_LOG_FILE_PATH);

    if !log_path.exists() {
        bail!("Log file not found at {}", SHERPAD_LOG_FILE_PATH);
    }

    if follow {
        // Use tail -f behavior: read existing content, then follow for new lines
        let file = fs::File::open(log_path)?;

        // First, print existing content
        let reader = BufReader::new(&file);
        for line in reader.lines() {
            println!("{}", line?);
        }

        // Now follow for new content
        loop {
            thread::sleep(Duration::from_millis(100));

            let reader = BufReader::new(&file);
            for line in reader.lines() {
                println!("{}", line?);
            }
        }
    } else {
        // Just print the entire file
        let contents = fs::read_to_string(log_path).context(format!(
            "Failed to read log file: {}",
            SHERPAD_LOG_FILE_PATH
        ))?;
        print!("{}", contents);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn_shell(script: &str) -> Child {
        Command::new("sh").arg("-c").arg(script).spawn().unwrap()
    }

    #[test]
    fn a_child_that_already_exited_is_reported() {
        let mut child = spawn_shell("exit 3");
        thread::sleep(Duration::from_millis(300));
        let error = check_still_running(&mut child).expect_err("an exited child passed");
        assert!(error.to_string().contains("exit status: 3"), "{error}");
    }

    #[test]
    fn a_running_child_passes() {
        let mut child = spawn_shell("sleep 5");
        assert!(check_still_running(&mut child).is_ok());
        child.kill().unwrap();
        child.wait().unwrap();
    }
}
