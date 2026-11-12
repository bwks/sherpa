use anyhow::{Context, Result, bail};
use bollard::Docker;
use bollard::query_parameters::InspectContainerOptions;
use bollard::secret::{ContainerState, ContainerStateStatusEnum};
use tracing::instrument;

use crate::status::is_missing;

/// Get the PID of a running container.
///
/// Returns the init PID of the container's main process,
/// which can be used to access the container's network namespace.
///
/// # Errors
///
/// Fails when the container cannot be inspected or is not running.
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
        .with_context(|| format!("no PID found for container {container_name}"))?;

    match u32::try_from(pid) {
        Ok(pid) if pid > 0 => Ok(pid),
        _ => bail!("container {container_name} has invalid PID {pid} — is it running?"),
    }
}

/// The current status of the container `container_name`, or `None` when Docker reports none.
///
/// # Errors
///
/// Fails when the container cannot be inspected.
#[instrument(skip(docker), fields(%container_name), level = "debug")]
pub async fn container_status(
    docker: &Docker,
    container_name: &str,
) -> Result<Option<ContainerStateStatusEnum>> {
    let details = docker
        .inspect_container(
            container_name,
            Some(InspectContainerOptions { size: false }),
        )
        .await
        .with_context(|| format!("failed to inspect container {container_name}"))?;
    Ok(details.state.and_then(|state| state.status))
}

/// The current state of the container `container_name`: its status, and how it ended when it
/// is not running. `None` when the container does not exist or Docker reports no state.
///
/// # Errors
///
/// Fails when the container cannot be inspected for another reason.
#[instrument(skip(docker), fields(%container_name), level = "debug")]
pub async fn container_state(
    docker: &Docker,
    container_name: &str,
) -> Result<Option<ContainerState>> {
    match docker
        .inspect_container(
            container_name,
            Some(InspectContainerOptions { size: false }),
        )
        .await
    {
        Ok(details) => Ok(details.state),
        Err(error) if is_missing(&error) => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("failed to inspect container {container_name}"))
        }
    }
}

/// The host path mounted at `destination` in a container, if one is.
///
/// # Errors
///
/// Fails when the container cannot be inspected.
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
