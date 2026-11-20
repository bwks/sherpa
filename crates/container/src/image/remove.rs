//! Removing an image Sherpa recorded from the local Docker daemon.

use anyhow::{Context, Result};
use bollard::Docker;
use bollard::models::ContainerSummary;
use bollard::query_parameters::{ListContainersOptions, RemoveImageOptions};
use tracing::instrument;

use crate::status::is_missing;

/// The running containers that use the image `reference` names, by name: those created from the
/// reference itself, and, when it is the image's last tag, every one running the image, as
/// removing the tag then removes the image. A container created from another tag of the same
/// image does not use a tag that is not the last one: removing it only untags. An image Docker
/// does not have has none.
///
/// # Errors
///
/// Fails when Docker cannot be reached, or cannot inspect the image or list the containers.
#[instrument(skip(docker), level = "debug")]
pub async fn running_containers_of_image(docker: &Docker, reference: &str) -> Result<Vec<String>> {
    let image = match docker.inspect_image(reference).await {
        Ok(image) => image,
        Err(error) if is_missing(&error) => return Ok(Vec::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to inspect container image {reference}"));
        }
    };
    let last_tag = image.repo_tags.as_ref().is_none_or(|tags| tags.len() <= 1);
    let Some(image_id) = image.id else {
        return Ok(Vec::new());
    };
    // Running containers only: Docker lists those unless `all` is set.
    let running = docker
        .list_containers(Some(ListContainersOptions::default()))
        .await
        .context("Failed to list running containers")?;
    Ok(containers_using(&running, reference, &image_id, last_tag))
}

/// The names of `containers`, without Docker's leading `/`, that use `reference`, a tag of the
/// image `image_id`: created from `reference`, or running the image at all when `last_tag`.
fn containers_using(
    containers: &[ContainerSummary],
    reference: &str,
    image_id: &str,
    last_tag: bool,
) -> Vec<String> {
    containers
        .iter()
        .filter(|container| {
            container.image.as_deref() == Some(reference)
                || (last_tag && container.image_id.as_deref() == Some(image_id))
        })
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

    fn container(name: &str, image: &str, image_id: &str) -> ContainerSummary {
        ContainerSummary {
            id: Some(format!("id-{name}")),
            names: Some(vec![format!("/{name}")]),
            image: Some(image.to_owned()),
            image_id: Some(image_id.to_owned()),
            ..Default::default()
        }
    }

    #[test]
    fn image_delete_003_another_tag_does_not_stop_the_delete_unless_it_is_the_images_last() {
        let last_tag = [
            container("ceos1-lab", "localrepo/arista_eos:4.35", "sha256:aaa"),
            container("frr1-lab", "frr:10.5", "sha256:bbb"),
            container("ceos2-lab", "sha256:aaa", "sha256:aaa"),
        ];
        assert_eq!(
            containers_using(&last_tag, "localrepo/arista_eos:4.35", "sha256:aaa", true),
            ["ceos1-lab", "ceos2-lab"]
        );
        assert_eq!(
            containers_using(&last_tag, "other:1", "sha256:ccc", true),
            Vec::<String>::new()
        );

        let other_tags = [
            container("frr1-lab", "quay.io/frrouting/frr:10.5.2", "sha256:aaa"),
            container("frr2-lab", "quay.io/frrouting/frr:10.5.2-alt", "sha256:aaa"),
        ];
        assert_eq!(
            containers_using(
                &other_tags,
                "quay.io/frrouting/frr:10.5.2-alt",
                "sha256:aaa",
                false
            ),
            ["frr2-lab"]
        );
    }
}
