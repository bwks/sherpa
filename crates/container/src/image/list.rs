use std::collections::HashMap;

use anyhow::{Context, Result};
use bollard::Docker;
use bollard::query_parameters::ListImagesOptions;
use tracing::instrument;

/// The `repo:tag`s of local container images, sorted: all of them, or with a `reference`, only
/// those Docker matches to it.
///
/// Docker does the matching, by its `reference` list filter: `mysql` and
/// `docker.io/library/mysql` both match `mysql:8.4`, `nats` does not match `natsio/nats-box`, and
/// `docker.io/mysql` matches nothing, as with `docker images`. Docker lists its names as it
/// tags them, so a match need not be spelled like `reference`.
///
/// # Errors
///
/// Fails when Docker cannot list the images.
#[instrument(skip(docker_conn), level = "debug")]
pub async fn get_local_images(
    docker_conn: &Docker,
    reference: Option<&str>,
) -> Result<Vec<String>> {
    let filters = reference
        .map(|reference| HashMap::from([("reference".to_string(), vec![reference.to_string()])]));
    let container_images = docker_conn
        .list_images(Some(ListImagesOptions {
            all: true,
            filters,
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
