use anyhow::{Context, Result};
use bollard::Docker;
use tracing::instrument;

use crate::status::is_missing;

/// Whether the local Docker daemon has the image `reference`. Docker resolves the reference
/// itself, so `docker.io/library/nats:2.11` finds the image it tags `nats:2.11`.
///
/// # Errors
///
/// Fails when Docker cannot be reached or answers with an error other than "not found".
#[instrument(skip(docker_conn), level = "debug")]
pub async fn image_exists(docker_conn: &Docker, reference: &str) -> Result<bool> {
    match docker_conn.inspect_image(reference).await {
        Ok(_) => Ok(true),
        Err(error) if is_missing(&error) => Ok(false),
        Err(error) => {
            Err(error).with_context(|| format!("Failed to inspect container image {reference}"))
        }
    }
}
