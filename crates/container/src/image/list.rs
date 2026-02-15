use anyhow::Result;
use bollard::Docker;
use bollard::query_parameters::ListImagesOptions;

/// List all container images
pub async fn list_images(docker_conn: &Docker) -> Result<()> {
    let container_images = docker_conn
        .list_images(Some(ListImagesOptions {
            all: true,
            ..Default::default()
        }))
        .await?;

    let mut image_list = vec![];
    for image in container_images {
        for tag in image.repo_tags {
            image_list.push(tag)
        }
    }
    image_list.sort();

    for image in image_list {
        tracing::info!(image = %image, "Container image");
    }

    Ok(())
}
