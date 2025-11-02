use std::process::Command;

use anyhow::Result;
use bollard::query_parameters::CreateImageOptionsBuilder;

use async_compression::tokio::write::GzipEncoder;
use async_compression::Level;
use bollard::Docker;
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

use crate::core::konst::{CONTAINER_IMAGE_NAME, TEMP_DIR};
use crate::core::Config;
use crate::data::ContainerImage;
use crate::util::{create_dir, dir_exists};

/// Pull down a container image from an OCI compliant Repository.
pub async fn pull_container_image(config: &Config, image: &ContainerImage) -> Result<()> {
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
    let file = tokio::fs::File::create(&format!("{}", image_save_location)).await?;
    let mut encoder = GzipEncoder::with_quality(file, Level::Fastest);

    while let Some(chunk) = export_stream.next().await {
        let chunk = chunk?;
        encoder.write_all(&chunk).await?;
    }
    encoder.shutdown().await?;

    println!("Image saved to: {}", image_save_location);

    Ok(())
}

/// Save a local container image the ".tmp/" directory.
pub fn save_container_image(image: &str, version: &str) -> Result<()> {
    let image_name = format!("{image}:{version}");
    println!("Exporting container image: {image_name}");
    if !dir_exists(TEMP_DIR) {
        create_dir(TEMP_DIR)?;
    }
    Command::new("docker")
        .args([
            "image",
            "save",
            "-o",
            &format!("{TEMP_DIR}/{CONTAINER_IMAGE_NAME}"),
            &image_name,
        ])
        .status()?;
    Ok(())
}
