use anyhow::{Context, Result, anyhow};
use bollard::Docker;
use bollard::query_parameters::InspectContainerOptions;
use tracing::instrument;

/// Get the PID of a running container.
///
/// Returns the init PID of the container's main process,
/// which can be used to access the container's network namespace.
#[instrument(skip(docker), fields(%container_name), level = "debug")]
pub async fn get_container_pid(docker: &Docker, container_name: &str) -> Result<u32> {
    let options = Some(InspectContainerOptions { size: false });
    let details = docker
        .inspect_container(container_name, options)
        .await
        .with_context(|| format!("failed to inspect container {container_name}"))?;

    let pid = details
        .state
        .as_ref()
        .and_then(|s| s.pid)
        .ok_or_else(|| anyhow!("no PID found for container {container_name}"))?;

    if pid <= 0 {
        return Err(anyhow!(
            "container {container_name} has invalid PID {pid} — is it running?"
        ));
    }

    Ok(pid as u32)
}

/// The host path mounted at `destination` in a container, if one is.
#[instrument(skip(docker), fields(%container_name, %destination), level = "debug")]
pub async fn get_container_mount_source(
    docker: &Docker,
    container_name: &str,
    destination: &str,
) -> Result<Option<String>> {
    let options = Some(InspectContainerOptions { size: false });
    let details = docker
        .inspect_container(container_name, options)
        .await
        .with_context(|| format!("failed to inspect container {container_name}"))?;
    for mount in details.mounts.unwrap_or_default() {
        if mount.destination.as_deref() == Some(destination) {
            return Ok(mount.source);
        }
    }
    Ok(None)
}
