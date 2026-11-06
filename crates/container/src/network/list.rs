use anyhow::{Context, Result};
use bollard::Docker;
use bollard::models::Network;
use bollard::query_parameters::ListNetworksOptions;
use tracing::instrument;

/// All Docker networks.
///
/// # Errors
///
/// Fails when Docker cannot list them.
#[instrument(skip(docker_conn), level = "debug")]
pub async fn list_networks(docker_conn: &Docker) -> Result<Vec<Network>> {
    docker_conn
        .list_networks(None::<ListNetworksOptions>)
        .await
        .context("Failed to list Docker networks")
}
