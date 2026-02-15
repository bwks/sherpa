use anyhow::{Context, Result, bail};
use nix::sys::signal::Signal;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use super::pidfile::{
    ensure_log_dir, ensure_run_dir, is_process_running, read_pid, remove_pid, send_signal,
    verify_not_running, write_pid,
};
use super::server::run_server;
use shared::konst::{
    SHERPA_BASE_DIR, SHERPA_LOG_DIR, SHERPA_RUN_DIR, SHERPAD_LOG_FILE, SHERPAD_PID_FILE,
};

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
        write_pid(&format!(
            "{SHERPA_BASE_DIR}/{SHERPA_RUN_DIR}/{SHERPAD_PID_FILE}"
        ))?;

        // Run server and clean up PID file on exit
        let result = run_server(true).await;

        remove_pid(&format!(
            "{SHERPA_BASE_DIR}/{SHERPA_RUN_DIR}/{SHERPAD_PID_FILE}"
        ))?;

        result
    } else {
        // Background mode: spawn as child process
        tracing::info!("Starting sherpad in background mode");

        // Spawn sherpad in a detached child process using nohup pattern
        let exe = std::env::current_exe()?;

        // We'll use a simple fork-like approach by spawning ourselves with a special flag
        // The child process will write its own PID
        let child = Command::new(exe)
            .arg("--background-child")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to spawn background process")?;

        let child_pid = child.id();

        // Give the child a moment to start and write its PID
        thread::sleep(Duration::from_millis(500));

        // Verify the child is still running
        if !is_process_running(child_pid) {
            bail!("Failed to start sherpad: process exited immediately");
        }

        tracing::info!(pid = child_pid, "sherpad started successfully");
        Ok(())
    }
}

/// Internal function to run as background child (called by start_daemon)
pub async fn run_background_child() -> Result<()> {
    // Write our own PID
    write_pid(&format!(
        "{SHERPA_BASE_DIR}/{SHERPA_RUN_DIR}/{SHERPAD_PID_FILE}"
    ))?;

    // Run server and clean up on exit
    let result = run_server(false).await;

    remove_pid(&format!(
        "{SHERPA_BASE_DIR}/{SHERPA_RUN_DIR}/{SHERPAD_PID_FILE}"
    ))?;

    result
}

/// Stop the sherpad daemon
pub fn stop_daemon(force: bool) -> Result<()> {
    // Read PID from file
    let pid = match read_pid(&format!(
        "{SHERPA_BASE_DIR}/{SHERPA_RUN_DIR}/{SHERPAD_PID_FILE}"
    ))? {
        Some(pid) => pid,
        None => {
            bail!("Server is not running");
        }
    };

    // Check if process is actually running
    if !is_process_running(pid) {
        tracing::warn!(pid = pid, "Server is not running (stale PID file found)");
        remove_pid(&format!(
            "{SHERPA_BASE_DIR}/{SHERPA_RUN_DIR}/{SHERPAD_PID_FILE}"
        ))?;
        return Ok(());
    }

    tracing::info!(pid = pid, "Stopping sherpad");

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
                remove_pid(&format!(
                    "{SHERPA_BASE_DIR}/{SHERPA_RUN_DIR}/{SHERPAD_PID_FILE}"
                ))?;
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

    remove_pid(&format!(
        "{SHERPA_BASE_DIR}/{SHERPA_RUN_DIR}/{SHERPAD_PID_FILE}"
    ))?;
    tracing::info!("sherpad stopped successfully");
    Ok(())
}

/// Restart the sherpad daemon
pub async fn restart_daemon(foreground: bool) -> Result<()> {
    tracing::info!("Restarting sherpad");

    // Stop the daemon if it's running
    if let Some(pid) = read_pid(&format!(
        "{SHERPA_BASE_DIR}/{SHERPA_RUN_DIR}/{SHERPAD_PID_FILE}"
    ))? {
        if is_process_running(pid) {
            stop_daemon(false)?;
            // Give it a moment to fully stop
            thread::sleep(Duration::from_millis(2000));
        } else {
            // Clean up stale PID file
            tracing::debug!("Found stale PID file, cleaning up");
            remove_pid(&format!(
                "{SHERPA_BASE_DIR}/{SHERPA_RUN_DIR}/{SHERPAD_PID_FILE}"
            ))?;
        }
    }

    // Start the daemon
    start_daemon(foreground).await
}

/// Show the status of the sherpad daemon
pub fn status_daemon() -> Result<()> {
    match read_pid(&format!(
        "{SHERPA_BASE_DIR}/{SHERPA_RUN_DIR}/{SHERPAD_PID_FILE}"
    ))? {
        Some(pid) => {
            if is_process_running(pid) {
                tracing::info!(pid = pid, "sherpad is running");
                Ok(())
            } else {
                tracing::warn!("sherpad is not running (stale PID file found)");
                std::process::exit(1);
            }
        }
        None => {
            tracing::info!("sherpad is not running");
            std::process::exit(1);
        }
    }
}

/// Show the logs of the sherpad daemon
pub fn logs_daemon(follow: bool) -> Result<()> {
    let path = format!("{SHERPA_BASE_DIR}/{SHERPA_LOG_DIR}/{SHERPAD_LOG_FILE}");
    let log_path = Path::new(&path);

    if !log_path.exists() {
        bail!("Log file not found at {}", &path);
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
            &format!("{SHERPA_BASE_DIR}/{SHERPA_LOG_DIR}/{SHERPAD_LOG_FILE}")
        ))?;
        print!("{}", contents);
        Ok(())
    }
}
