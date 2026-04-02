use anyhow::{Context, Result};
use bollard::Docker;
use bollard::query_parameters::StartContainerOptions;
use tracing::instrument;

#[instrument(skip(docker), level = "debug")]
pub async fn start_container(docker: &Docker, name: &str) -> Result<()> {
    docker
        .start_container(name, None::<StartContainerOptions>)
        .await
        .with_context(|| format!("Failed to start container: {name}"))?;

    tracing::info!(container_name = %name, "Started container");
    Ok(())
}

#[instrument(skip(docker), level = "debug")]
pub async fn unpause_container(docker: &Docker, name: &str) -> Result<()> {
    docker
        .unpause_container(name)
        .await
        .with_context(|| format!("Failed to unpause container: {name}"))?;

    tracing::info!(container_name = %name, "Unpaused container");
    Ok(())
}
