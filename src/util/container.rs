use std::process::Command;

use anyhow::Result;

use super::file_system::{create_dir, dir_exists};
use crate::core::konst::{CONTAINER_IMAGE_NAME, TEMP_DIR};

/// Pull a container image
pub fn pull_container_image(image: &str) -> Result<()> {
    println!("Pulling container image: {image}");
    Command::new("docker")
        .args(["image", "pull", image])
        .status()?;
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
