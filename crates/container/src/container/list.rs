use anyhow::{Context, Result};
use bollard::Docker;
use bollard::models::ContainerSummary;
use bollard::query_parameters::ListContainersOptions;
use tracing::instrument;

/// All containers, running or not.
///
/// # Errors
///
/// Fails when Docker cannot list them.
#[instrument(skip(docker_conn), level = "debug")]
pub async fn list_containers(docker_conn: &Docker) -> Result<Vec<ContainerSummary>> {
    let options = Some(ListContainersOptions {
        all: true,
        ..Default::default()
    });
    docker_conn
        .list_containers(options)
        .await
        .context("Failed to list containers")
}
