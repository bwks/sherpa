use anyhow::{Context, Result};
use bollard::Docker;
use bollard::query_parameters::{RestartContainerOptions, StartContainerOptions};
use tracing::instrument;

/// Start the stopped container `name`.
///
/// # Errors
///
/// Fails when the container does not exist or Docker cannot start it.
#[instrument(skip(docker), level = "debug")]
pub async fn start_container(docker: &Docker, name: &str) -> Result<()> {
    docker
        .start_container(name, None::<StartContainerOptions>)
        .await
        .with_context(|| format!("Failed to start container: {name}"))?;

    tracing::info!(container_name = %name, "Started container");
    Ok(())
}

/// Restart the container `name`: stop it, waiting for it as Docker's default stop does, and start
/// it again. Its mounts are bound again, so it reads files replaced since it started.
///
/// # Errors
///
/// Fails when the container does not exist or Docker cannot restart it.
#[instrument(skip(docker), level = "debug")]
pub async fn restart_container(docker: &Docker, name: &str) -> Result<()> {
    docker
        .restart_container(name, None::<RestartContainerOptions>)
        .await
        .with_context(|| format!("Failed to restart container: {name}"))?;

    tracing::info!(container_name = %name, "Restarted container");
    Ok(())
}

/// Resume the paused container `name`.
///
/// # Errors
///
/// Fails when the container does not exist or is not paused.
#[instrument(skip(docker), level = "debug")]
pub async fn unpause_container(docker: &Docker, name: &str) -> Result<()> {
    docker
        .unpause_container(name)
        .await
        .with_context(|| format!("Failed to unpause container: {name}"))?;

    tracing::info!(container_name = %name, "Unpaused container");
    Ok(())
}
