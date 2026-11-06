use anyhow::{Context, Result};
use bollard::Docker;
use tracing::instrument;

use crate::status::is_missing;

/// Delete the Docker network `name`. A network that is already gone counts as deleted, so a
/// retried destroy succeeds.
///
/// # Errors
///
/// Fails when Docker refuses, as it does while containers are attached to the network.
#[instrument(skip(docker), level = "debug")]
pub async fn delete_network(docker: &Docker, name: &str) -> Result<()> {
    match docker.remove_network(name).await {
        Ok(()) => {
            tracing::info!(network_name = %name, "Destroyed container network");
            Ok(())
        }
        Err(error) if is_missing(&error) => {
            tracing::debug!(network_name = %name, "Container network already destroyed");
            Ok(())
        }
        Err(error) => {
            Err(error).with_context(|| format!("Failed to delete Docker network '{name}'"))
        }
    }
}
