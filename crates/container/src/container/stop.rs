use anyhow::{Context, Result};
use bollard::Docker;
use bollard::query_parameters::StopContainerOptions;
use tracing::instrument;

#[instrument(skip(docker), level = "debug")]
pub async fn stop_container(docker: &Docker, name: &str) -> Result<()> {
    docker
        .stop_container(name, None::<StopContainerOptions>)
        .await
        .with_context(|| format!("Failed to stop container: {name}"))?;

    tracing::info!(container_name = %name, "Stopped container");
    Ok(())
}

#[instrument(skip(docker), level = "debug")]
pub async fn pause_container(docker: &Docker, name: &str) -> Result<()> {
    docker
        .pause_container(name)
        .await
        .with_context(|| format!("Failed to pause container: {name}"))?;

    tracing::info!(container_name = %name, "Paused container");
    Ok(())
}
