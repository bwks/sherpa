use anyhow::{Context, Result};
use bollard::query_parameters::CreateImageOptionsBuilder;
use futures_util::StreamExt;
use tracing::instrument;

use crate::connect::docker_connection;
use crate::image::reference::image_reference;

/// Pull a container image from an OCI registry into the local Docker daemon, like
/// `docker pull`.
///
/// The `on_progress` callback is invoked with human-readable status strings as the pull
/// progresses.
///
/// # Errors
///
/// Fails when Docker cannot be reached or the pull fails.
#[instrument(skip(on_progress), level = "debug")]
pub async fn pull_image<F>(repo: &str, tag: &str, on_progress: F) -> Result<()>
where
    F: Fn(&str),
{
    let docker = docker_connection()?;

    let image_location = image_reference(repo, tag);

    tracing::info!(image = %image_location, "Pulling container image");
    on_progress(&format!("Pulling {image_location}..."));

    let options = CreateImageOptionsBuilder::default()
        .from_image(repo)
        .tag(tag)
        .build();

    // The pull saves directly to Docker's local image store.
    let mut pull_stream = docker.create_image(Some(options), None, None);

    while let Some(pull_result) = pull_stream.next().await {
        let info = pull_result.with_context(|| format!("Error pulling image {image_location}"))?;
        if let Some(ref status) = info.status {
            if let Some(ref progress) = info.progress {
                on_progress(&format!("{status}: {progress}"));
                tracing::debug!(status = %status, progress = %progress, "Image pull progress");
            } else {
                on_progress(status);
                tracing::debug!(status = %status, "Image pull progress");
            }
        }
    }

    tracing::info!(image = %image_location, "Successfully pulled image, now available in local Docker daemon");
    on_progress(&format!("Successfully pulled {image_location}"));
    Ok(())
}
