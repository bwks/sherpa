use anyhow::{Context, Result, bail};
use nix::sys::signal::Signal;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{self, Child, Command, Stdio};
use std::thread;
use std::time::Duration;

use super::pidfile::{
    SHERPAD_LOG_FILE_PATH, SHERPAD_PID_FILE_PATH, ensure_log_dir, ensure_run_dir,
    is_process_running, lock_pid_file, open_log_file, read_pid, remove_pid, running_server_pid,
    send_signal, verify_not_running,
};
use super::server::run_server;
use crate::konst::SHERPA_CONFIG_FILE_PATH;
use crate::tls::{CertificateManager, CertificateReport, server_addresses};
use shared::util::load_config;

/// Start the sherpad daemon
///
/// # Errors
///
/// Fails when the run or log directory cannot be created, sherpad is already running, the PID
/// file cannot be locked or written, the background process cannot be started or exits at once,
/// or the server fails in the foreground.
pub async fn start_daemon(foreground: bool) -> Result<()> {
    // Ensure directories exist
    ensure_run_dir()?;
    ensure_log_dir()?;

    // Check if already running
    verify_not_running()?;

    if foreground {
        // Foreground mode: run server directly in current process
        tracing::info!("Starting sherpad in foreground mode");
        let _pid_lock = lock_pid_file(SHERPAD_PID_FILE_PATH)?;

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
        let stderr = open_log_file(Path::new(SHERPAD_LOG_FILE_PATH))?;
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

/// Internal function to run as background child (called by `start_daemon`)
///
/// # Errors
///
/// Fails when the PID file cannot be locked, written or removed, or the server fails.
pub async fn run_background_child() -> Result<()> {
    let _pid_lock = lock_pid_file(SHERPAD_PID_FILE_PATH)?;

    // Run server and clean up on exit
    let result = run_server(false).await;

    remove_pid(SHERPAD_PID_FILE_PATH)?;

    result
}

/// Stop the sherpad daemon the PID file at `pid_path` names. A PID whose process is gone or is
/// not `sherpad` is never signalled: the stale file is removed.
///
/// # Errors
///
/// Fails when sherpad is not running, the PID file cannot be read or removed, or the process
/// cannot be signalled or does not exit.
pub fn stop_daemon(pid_path: &str, force: bool) -> Result<()> {
    if read_pid(pid_path)?.is_none() {
        bail!("Server is not running");
    }
    let Some(pid) = running_server_pid(pid_path)? else {
        tracing::warn!("Server is not running (stale PID file found)");
        remove_pid(pid_path)?;
        return Ok(());
    };

    tracing::info!(%pid, "Stopping sherpad");

    // Send SIGTERM for graceful shutdown
    if let Err(e) = send_signal(pid, Signal::SIGTERM) {
        if force {
            tracing::warn!(error = %e, "Failed to send SIGTERM, trying SIGKILL");
        } else {
            bail!("Failed to stop server: {e}");
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
                remove_pid(pid_path)?;
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

    remove_pid(pid_path)?;
    tracing::info!("sherpad stopped successfully");
    Ok(())
}

/// Restart the sherpad daemon
///
/// # Errors
///
/// Fails when the running server cannot be stopped or the new one cannot be started; see
/// [`stop_daemon`] and [`start_daemon`].
pub async fn restart_daemon(foreground: bool) -> Result<()> {
    tracing::info!("Restarting sherpad");

    if running_server_pid(SHERPAD_PID_FILE_PATH)?.is_some() {
        stop_daemon(SHERPAD_PID_FILE_PATH, false)?;
        // Give it a moment to fully stop
        thread::sleep(Duration::from_millis(2000));
    }

    // Start the daemon
    start_daemon(foreground).await
}

/// Show the status of the sherpad daemon. Exits with status 1 when it is not running.
///
/// # Errors
///
/// Fails when the PID file cannot be read.
pub fn status_daemon() -> Result<()> {
    if let Some(pid) = running_server_pid(SHERPAD_PID_FILE_PATH)? {
        println!("sherpad is running (pid: {pid})");
        print_certificate_status();
        Ok(())
    } else if read_pid(SHERPAD_PID_FILE_PATH)?.is_some() {
        eprintln!("sherpad is not running (stale PID file found)");
        process::exit(1);
    } else {
        eprintln!("sherpad is not running");
        process::exit(1);
    }
}

/// Print the server certificate's path, key pin and expiry, and any warning about a certificate
/// the admin provides. A certificate that cannot be read is reported, and status still succeeds.
fn print_certificate_status() {
    let config = match load_config(SHERPA_CONFIG_FILE_PATH) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("Cannot read {SHERPA_CONFIG_FILE_PATH} for the certificate: {error:#}");
            return;
        }
    };
    if !config.tls.enabled {
        println!("TLS is off");
        return;
    }
    let manager = CertificateManager::new(&config.tls);
    match manager.report(&server_addresses(&config), jiff::Timestamp::now()) {
        Ok(report) => {
            for line in certificate_status_lines(&report) {
                println!("{line}");
            }
        }
        Err(error) => eprintln!("Cannot read the server certificate: {error:#}"),
    }
}

/// The lines `sherpad status` prints about the server certificate.
fn certificate_status_lines(report: &CertificateReport) -> Vec<String> {
    let owner = if report.self_generated {
        "written by sherpad at each start"
    } else {
        "provided by the admin"
    };
    let mut lines = vec![
        format!("Certificate: {} ({owner})", report.cert_path.display()),
        format!("Key pin (SHA-256 of the public key): {}", report.key_pin),
        format!("Valid until: {}", report.not_after),
    ];
    lines.extend(
        report
            .warnings
            .iter()
            .map(|warning| format!("Warning: {warning}")),
    );
    lines
}

/// Show the logs of the sherpad daemon
///
/// # Errors
///
/// Fails when the log file does not exist or cannot be read.
pub fn logs_daemon(follow: bool) -> Result<()> {
    let log_path = Path::new(SHERPAD_LOG_FILE_PATH);

    if !log_path.exists() {
        bail!("Log file not found at {SHERPAD_LOG_FILE_PATH}");
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
    }
    // Just print the entire file
    let contents = fs::read_to_string(log_path)
        .context(format!("Failed to read log file: {SHERPAD_LOG_FILE_PATH}"))?;
    print!("{contents}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::pidfile::tests::spawn_sherpad_stand_in;
    use nix::unistd::Pid;
    use tempfile::TempDir;

    fn spawn_shell(script: &str) -> Child {
        Command::new("sh").arg("-c").arg(script).spawn().unwrap()
    }

    #[test]
    fn sherpad_daemon_001_a_start_is_failed_when_the_server_exits_at_once_and_started_when_it_runs()
    {
        let mut exited = spawn_shell("exit 3");
        thread::sleep(Duration::from_millis(300));
        let error = check_still_running(&mut exited).expect_err("an exited child passed");
        assert!(error.to_string().contains("exit status: 3"), "{error}");

        let mut running = spawn_shell("sleep 5");
        assert!(check_still_running(&mut running).is_ok());
        running.kill().unwrap();
        running.wait().unwrap();
    }

    #[test]
    fn sherpad_daemon_012_a_pid_file_naming_a_process_that_is_not_sherpad_reads_as_no_server_and_is_never_signalled()
     {
        let temp_dir = TempDir::new().unwrap();
        let pid_file = temp_dir.path().join("sherpad.pid");
        let pid_path = pid_file.to_str().unwrap();

        let mut other = Command::new("sleep").arg("30").spawn().unwrap();
        fs::write(pid_path, other.id().to_string()).unwrap();
        assert_eq!(running_server_pid(pid_path).unwrap(), None);
        stop_daemon(pid_path, true).unwrap();
        assert!(
            other.try_wait().unwrap().is_none(),
            "the process was signalled"
        );
        assert!(!pid_file.exists());
        other.kill().unwrap();
        other.wait().unwrap();

        let mut server = spawn_sherpad_stand_in();
        let server_pid = Pid::from_raw(i32::try_from(server.id()).unwrap());
        fs::write(pid_path, server_pid.to_string()).unwrap();
        assert_eq!(running_server_pid(pid_path).unwrap(), Some(server_pid));
        server.kill().unwrap();
        server.wait().unwrap();
    }
}
