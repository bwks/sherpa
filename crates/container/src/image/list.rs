use anyhow::Result;
use bollard::Docker;
use bollard::query_parameters::ListImagesOptions;

/// Get list of all local container images with their tags
pub async fn get_local_images(docker_conn: &Docker) -> Result<Vec<String>> {
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

    Ok(image_list)
}

/// List all container images (logs to console)
pub async fn list_images(docker_conn: &Docker) -> Result<()> {
    let image_list = get_local_images(docker_conn).await?;

    for image in image_list {
        tracing::info!(image = %image, "Container image");
    }

    Ok(())
}
