use anyhow::{Context, Result};
use bollard::Docker;
use bollard::query_parameters::ListImagesOptions;
use tracing::instrument;

/// The tags of all local container images, sorted.
///
/// # Errors
///
/// Fails when Docker cannot list the images.
#[instrument(skip(docker_conn), level = "debug")]
pub async fn get_local_images(docker_conn: &Docker) -> Result<Vec<String>> {
    let container_images = docker_conn
        .list_images(Some(ListImagesOptions {
            all: true,
            ..Default::default()
        }))
        .await
        .context("Failed to list container images")?;

    let mut image_list: Vec<String> = container_images
        .into_iter()
        .flat_map(|image| image.repo_tags)
        .collect();
    image_list.sort();

    Ok(image_list)
}
