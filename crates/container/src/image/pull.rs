use anyhow::Result;
use async_compression::Level;
use async_compression::tokio::write::GzipEncoder;
use bollard::Docker;
use bollard::query_parameters::CreateImageOptionsBuilder;
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

use data::{Config as SherpaConfig, ContainerImage};

/// Pull a container image from an OCI registry and save to local Docker daemon
/// Similar to `docker pull` command
pub async fn pull_image(repo: &str, tag: &str) -> Result<()> {
    let docker = Docker::connect_with_local_defaults()?;

    let image_location = format!("{}:{}", repo, tag);

    println!("Pulling image: {}", image_location);

    // Specify the image details using the builder
    let options = CreateImageOptionsBuilder::default()
        .from_image(repo)
        .tag(tag)
        .build();

    // Pull the image - this saves directly to Docker's local image store
    let mut pull_stream = docker.create_image(Some(options), None, None);

    while let Some(pull_result) = pull_stream.next().await {
        match pull_result {
            Ok(info) => {
                // Optionally print pull progress
                if let Some(status) = info.status {
                    if let Some(progress) = info.progress {
                        println!("{}: {}", status, progress);
                    } else {
                        println!("{}", status);
                    }
                }
            }
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "Error pulling image {}: {}",
                    image_location,
                    e
                ));
            }
        }
    }

    println!("Successfully pulled image: {}", image_location);
    println!("Image is now available in local Docker daemon");
    Ok(())
}

/// Pull down a container image from an OCI compliant Repository.
pub async fn pull_container_image(config: &SherpaConfig, image: &ContainerImage) -> Result<()> {
    let image_location = format!("{}:{}", image.repo, image.version);
    let image_save_location = format!("{}/{}.tar.gz", config.containers_dir, image.name);

    let docker = Docker::connect_with_local_defaults()?;

    // Specify the image details using the new builder
    let options = CreateImageOptionsBuilder::default()
        .from_image(&image_location)
        .build();

    // Pull the image
    println!("Pulling image: {}", image.name);
    let mut pull_stream = docker.create_image(Some(options), None, None);
    while let Some(_pull_result) = pull_stream.next().await {}

    println!("Exporting image: {}", image.name);
    // Export the image and save as a .tar.gz
    let mut export_stream = docker.export_image(&image_location);

    println!("Saving image to: {}", image_save_location);
    let file = tokio::fs::File::create(&image_save_location).await?;
    let mut encoder = GzipEncoder::with_quality(file, Level::Fastest);

    while let Some(chunk) = export_stream.next().await {
        let chunk = chunk?;
        encoder.write_all(&chunk).await?;
    }
    encoder.shutdown().await?;

    println!("Image saved to: {}", image_save_location);

    Ok(())
}
