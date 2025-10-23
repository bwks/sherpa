use std::process::Command;

use anyhow::Result;

use flate2::{write::GzEncoder, Compression};
use oci_client::{
    client::{Client, ClientConfig, ClientProtocol},
    secrets::RegistryAuth,
    Reference,
};
use std::{fs::File, io::Write};

use crate::core::konst::{CONTAINER_IMAGE_NAME, TEMP_DIR};
use crate::core::Config;
use crate::data::ContainerImage;
use crate::util::{create_dir, dir_exists};

/// Pull down a container image from an OCI compliant Repository.
pub async fn pull_container_image(config: &Config, image: &ContainerImage) -> Result<()> {
    let client = Client::new(ClientConfig {
        protocol: ClientProtocol::Https,
        ..Default::default()
    });

    // Parse the image reference
    let reference: Reference = format!("{}:{}", image.repo, image.version).parse()?;

    // Media types we accept (standard OCI + Docker schema2)
    let accept = vec![
        oci_client::manifest::WASM_LAYER_MEDIA_TYPE,
        oci_client::manifest::WASM_CONFIG_MEDIA_TYPE,
        oci_client::manifest::IMAGE_MANIFEST_MEDIA_TYPE,
        oci_client::manifest::IMAGE_MANIFEST_LIST_MEDIA_TYPE,
        oci_client::manifest::OCI_IMAGE_INDEX_MEDIA_TYPE,
        oci_client::manifest::OCI_IMAGE_MEDIA_TYPE,
        oci_client::manifest::IMAGE_CONFIG_MEDIA_TYPE,
        oci_client::manifest::IMAGE_DOCKER_CONFIG_MEDIA_TYPE,
        oci_client::manifest::IMAGE_LAYER_MEDIA_TYPE,
        oci_client::manifest::IMAGE_LAYER_GZIP_MEDIA_TYPE,
        oci_client::manifest::IMAGE_DOCKER_LAYER_TAR_MEDIA_TYPE,
        oci_client::manifest::IMAGE_DOCKER_LAYER_GZIP_MEDIA_TYPE,
        oci_client::manifest::IMAGE_LAYER_NONDISTRIBUTABLE_MEDIA_TYPE,
        oci_client::manifest::IMAGE_LAYER_NONDISTRIBUTABLE_GZIP_MEDIA_TYPE,
    ];

    println!("Pulling {} ...", reference);

    // Correct function call: includes `accept` as the 4th parameter
    let container_image = client
        .pull(&reference, &RegistryAuth::Anonymous, accept)
        .await?;

    println!(
        "Manifest digest: {}",
        container_image.manifest.unwrap().config.digest
    );

    // Save all layers into one compressed tarball
    let tar_path = &format!("{}/{}.tar.gz", config.containers_dir, image.name);
    let file = File::create(tar_path)?;
    let mut encoder = GzEncoder::new(file, Compression::default());

    for layer in &container_image.layers {
        encoder.write_all(&layer.data)?;
    }

    encoder.finish()?;
    println!("Exported to {}", tar_path);
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
