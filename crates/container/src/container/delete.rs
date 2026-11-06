use anyhow::{Context, Result};
use bollard::Docker;
use bollard::query_parameters::{KillContainerOptions, RemoveContainerOptions};
use tracing::instrument;

use crate::status::is_missing;

/// Send `SIGKILL` to the container `name`.
///
/// # Errors
///
/// Fails when the container does not exist or is not running.
#[instrument(skip(docker), level = "debug")]
pub async fn kill_container(docker: &Docker, name: &str) -> Result<()> {
    docker
        .kill_container(
            name,
            Some(KillContainerOptions {
                signal: "SIGKILL".to_string(),
            }),
        )
        .await
        .with_context(|| format!("Error killing container: {name}"))?;

    tracing::info!(container_name = %name, "Killed container");
    Ok(())
}

/// Force-remove the container `name`, stopping it first if it runs. A container that is
/// already gone counts as removed, so a retried cleanup succeeds.
///
/// # Errors
///
/// Fails when Docker refuses to remove the container.
#[instrument(skip(docker), level = "debug")]
pub async fn remove_container(docker: &Docker, name: &str) -> Result<()> {
    match docker
        .remove_container(
            name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
    {
        Ok(()) => {
            tracing::info!(container_name = %name, "Removed container");
            Ok(())
        }
        Err(error) if is_missing(&error) => {
            tracing::debug!(container_name = %name, "Container already removed");
            Ok(())
        }
        Err(error) => Err(error).with_context(|| format!("Error removing container: {name}")),
    }
}
