use anyhow::{Context, Result};
use bollard::Docker;
use tracing::instrument;

/// A client for the local Docker daemon, over its default socket.
///
/// # Errors
///
/// Fails when the client cannot be set up for the local socket.
#[instrument(level = "debug")]
pub fn docker_connection() -> Result<Docker> {
    Docker::connect_with_local_defaults().context("Failed to connect to the Docker daemon")
}
