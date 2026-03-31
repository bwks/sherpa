use anyhow::{Context, Result, anyhow, bail};
use bollard::Docker;
use bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};
use futures_util::TryStreamExt;

/// Execute a command inside a running container in attached mode.
/// Waits for the command to complete and verifies the exit code.
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

    let exec_id = exec.id.clone();

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
        while output.try_next().await?.is_some() {}
    }

    // Check exit code via inspect.
    let inspect = docker
        .inspect_exec(&exec_id)
        .await
        .with_context(|| format!("Failed to inspect exec in container {container_name}"))?;

    match inspect.exit_code {
        Some(0) => Ok(()),
        Some(code) => bail!("Exec in container {container_name} exited with code {code}"),
        None => Err(anyhow!(
            "Exec in container {container_name} has no exit code (still running?)"
        )),
    }
}

/// Execute a command inside a running container in detached mode.
/// Fire-and-forget: the command starts but we do not wait for it to complete.
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

/// Execute a command inside a running container with retry logic.
/// Retries up to `max_retries` times with `delay` between attempts.
pub async fn exec_container_with_retry(
    docker: &Docker,
    container_name: &str,
    cmd: Vec<&str>,
    max_retries: u32,
    delay: std::time::Duration,
) -> Result<()> {
    let mut last_err = None;

    for attempt in 0..=max_retries {
        if attempt > 0 {
            tracing::debug!(
                container_name = %container_name,
                attempt = attempt,
                "Retrying exec after delay"
            );
            tokio::time::sleep(delay).await;
        }

        match exec_container(docker, container_name, cmd.clone()).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                tracing::warn!(
                    container_name = %container_name,
                    attempt = attempt + 1,
                    max_retries = max_retries + 1,
                    error = %e,
                    "Exec attempt failed"
                );
                last_err = Some(e);
            }
        }
    }

    Err(last_err
        .unwrap_or_else(|| anyhow::anyhow!("exec retry loop completed with no error captured")))
}
