//! Removing an image Sherpa recorded from the local Docker daemon.

use anyhow::{Context, Result};
use bollard::Docker;
use bollard::models::ContainerSummary;
use bollard::query_parameters::{ListContainersOptions, RemoveImageOptions};
use tracing::instrument;

use crate::status::is_missing;

/// The running containers created from the image `reference` names, by name. Docker resolves
/// the reference to an image ID, and a container matches by that ID, so a container created
/// from another tag of the same image is found too. An image Docker does not have has none.
///
/// # Errors
///
/// Fails when Docker cannot be reached, or cannot inspect the image or list the containers.
#[instrument(skip(docker), level = "debug")]
pub async fn running_containers_of_image(docker: &Docker, reference: &str) -> Result<Vec<String>> {
    let image_id = match docker.inspect_image(reference).await {
        Ok(image) => image.id,
        Err(error) if is_missing(&error) => return Ok(Vec::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to inspect container image {reference}"));
        }
    };
    let Some(image_id) = image_id else {
        return Ok(Vec::new());
    };
    // Running containers only: Docker lists those unless `all` is set.
    let running = docker
        .list_containers(Some(ListContainersOptions::default()))
        .await
        .context("Failed to list running containers")?;
    Ok(containers_of_image(&running, &image_id))
}

/// The names of `containers` created from the image `image_id`, without Docker's leading `/`.
fn containers_of_image(containers: &[ContainerSummary], image_id: &str) -> Vec<String> {
    containers
        .iter()
        .filter(|container| container.image_id.as_deref() == Some(image_id))
        .map(|container| {
            container
                .names
                .as_ref()
                .and_then(|names| names.first())
                .map_or_else(
                    || container.id.clone().unwrap_or_default(),
                    |name| name.trim_start_matches('/').to_owned(),
                )
        })
        .collect()
}

/// Remove the tag `reference` from the local Docker daemon, and the image when it was its last
/// tag. It is forced, so a stopped container created from the image does not keep it; the caller
/// refuses first when a running container runs it ([`running_containers_of_image`]). Returns
/// whether Docker had the tag: one already gone counts as removed.
///
/// # Errors
///
/// Fails when Docker cannot be reached or refuses the removal.
#[instrument(skip(docker))]
pub async fn remove_image(docker: &Docker, reference: &str) -> Result<bool> {
    let options = RemoveImageOptions {
        force: true,
        noprune: false,
    };
    match docker.remove_image(reference, Some(options), None).await {
        Ok(_) => {
            tracing::info!(image = %reference, "Removed container image");
            Ok(true)
        }
        Err(error) if is_missing(&error) => {
            tracing::debug!(image = %reference, "Container image already removed");
            Ok(false)
        }
        Err(error) => {
            Err(error).with_context(|| format!("Failed to remove container image {reference}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn container(name: &str, image_id: &str) -> ContainerSummary {
        ContainerSummary {
            id: Some(format!("id-{name}")),
            names: Some(vec![format!("/{name}")]),
            image_id: Some(image_id.to_owned()),
            ..Default::default()
        }
    }

    #[test]
    fn containers_are_matched_to_an_image_by_its_id() {
        let containers = [
            container("ceos1-lab", "sha256:aaa"),
            container("frr1-lab", "sha256:bbb"),
            container("ceos2-lab", "sha256:aaa"),
        ];
        assert_eq!(
            containers_of_image(&containers, "sha256:aaa"),
            ["ceos1-lab", "ceos2-lab"]
        );
        assert_eq!(
            containers_of_image(&containers, "sha256:ccc"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_container_without_a_name_is_named_by_its_id() {
        let unnamed = ContainerSummary {
            names: None,
            ..container("x", "sha256:aaa")
        };
        assert_eq!(containers_of_image(&[unnamed], "sha256:aaa"), ["id-x"]);
    }
}
