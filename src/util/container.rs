use std::process::Command;

use anyhow::Result;

use crate::core::konst::CONTAINER_IMAGE_NAME;

/// Pull a container image
pub fn pull_container_image(image: &str) -> Result<()> {
    println!("Pulling container image: {image}");
    Command::new("docker")
        .args(["image", "pull", image])
        .status()?;
    Ok(())
}

/// Save a container image
pub fn save_container_image(image: &str, version: &str) -> Result<()> {
    let image_name = format!("{image}:{version}");
    println!("Exporting container image: {image_name}");
    Command::new("docker")
        .args(["image", "save", "-o", CONTAINER_IMAGE_NAME, &image_name])
        .status()?;
    Ok(())
}
