use std::time::Duration;

use anyhow::{Context, Result, bail};
use bollard::Docker;
use bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};
use futures_util::TryStreamExt;
use tokio::time::sleep;
use tracing::instrument;

/// Execute a command inside a running container in attached mode.
/// Waits for the command to complete and verifies the exit code.
///
/// # Errors
///
/// Fails when the command cannot be run, or exits with a non-zero code.
#[instrument(skip(docker), level = "debug")]
pub async fn exec_container(docker: &Docker, container_name: &str, cmd: Vec<&str>) -> Result<()> {
    let config = CreateExecOptions {
        cmd: Some(cmd),
        user: Some("root"),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        ..Default::default()
    };

    let exec = docker
        .create_exec(container_name, config)
        .await
        .with_context(|| format!("Failed to create exec in container {container_name}"))?;

    let result = docker
        .start_exec(
            &exec.id,
            Some(StartExecOptions {
                detach: false,
                ..Default::default()
            }),
        )
        .await
        .with_context(|| format!("Failed to start exec in container {container_name}"))?;

    // Drain the output stream to wait for the command to finish.
    if let StartExecResults::Attached { mut output, .. } = result {
        while output
            .try_next()
            .await
            .with_context(|| format!("Failed to read exec output in container {container_name}"))?
            .is_some()
        {}
    }

    // Check exit code via inspect.
    let inspect = docker
        .inspect_exec(&exec.id)
        .await
        .with_context(|| format!("Failed to inspect exec in container {container_name}"))?;

    match inspect.exit_code {
        Some(0) => Ok(()),
        Some(code) => bail!("Exec in container {container_name} exited with code {code}"),
        None => bail!("Exec in container {container_name} has no exit code (still running?)"),
    }
}

/// Execute a command inside a running container in detached mode.
/// Fire-and-forget: the command starts but we do not wait for it to complete.
///
/// # Errors
///
/// Fails when the command cannot be started.
#[instrument(skip(docker), level = "debug")]
pub async fn exec_container_detached(
    docker: &Docker,
    container_name: &str,
    cmd: Vec<&str>,
) -> Result<()> {
    let config = CreateExecOptions {
        cmd: Some(cmd),
        user: Some("root"),
        ..Default::default()
    };

    let exec = docker
        .create_exec(container_name, config)
        .await
        .with_context(|| format!("Failed to create exec in container {container_name}"))?;

    docker
        .start_exec(
            &exec.id,
            Some(StartExecOptions {
                detach: true,
                ..Default::default()
            }),
        )
        .await
        .with_context(|| format!("Failed to start exec in container {container_name}"))?;

    Ok(())
}

/// Execute a command inside a running container, retrying up to `max_retries` times with
/// `delay` between attempts.
///
/// # Errors
///
/// Fails with the last attempt's error when every attempt fails.
#[instrument(skip(docker), level = "debug")]
pub async fn exec_container_with_retry(
    docker: &Docker,
    container_name: &str,
    cmd: Vec<&str>,
    max_retries: u32,
    delay: Duration,
) -> Result<()> {
    let mut attempt = 0;
    loop {
        match exec_container(docker, container_name, cmd.clone()).await {
            Ok(()) => return Ok(()),
            Err(error) if attempt < max_retries => {
                attempt += 1;
                tracing::warn!(
                    container_name = %container_name,
                    attempt,
                    max_retries,
                    error = %error,
                    "Exec attempt failed; retrying"
                );
                sleep(delay).await;
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Exec in container {container_name} failed after {} attempts",
                        max_retries + 1
                    )
                });
            }
        }
    }
}
