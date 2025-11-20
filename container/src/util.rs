use std::collections::HashMap;
use std::process::Command;

use anyhow::{Context, Result};
use async_compression::Level;
use async_compression::tokio::write::GzipEncoder;
use bollard::Docker;
use bollard::models::{
    ContainerCreateBody, ContainerCreateResponse, ContainerSummary, EndpointIpamConfig,
    EndpointSettings, HostConfig, Ipam, IpamConfig, Network, NetworkCreateRequest,
    NetworkingConfig,
};
use bollard::query_parameters::{
    CreateContainerOptions, CreateImageOptionsBuilder, InspectContainerOptions,
    KillContainerOptions, ListContainersOptions, ListNetworksOptions, RemoveContainerOptions,
    StartContainerOptions,
};
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

use data::{Config as SherpaConfig, ContainerImage, ContainerNetworkAttachment};
use konst::{CONTAINER_IMAGE_NAME, TEMP_DIR};
use util::{create_dir, dir_exists};

pub fn docker_connection() -> Result<Docker> {
    let docker = Docker::connect_with_local_defaults()?;
    Ok(docker)
}

pub async fn list_containers(docker_conn: &Docker) -> Result<Vec<ContainerSummary>> {
    let options = Some(ListContainersOptions {
        all: true,
        ..Default::default()
    });
    Ok(docker_conn.list_containers(options).await?)
}

pub async fn list_networks(docker_conn: &Docker) -> Result<Vec<Network>> {
    let options = Some(ListNetworksOptions {
        ..Default::default()
    });
    Ok(docker_conn.list_networks(options).await?)
}
pub async fn create_network(
    docker: &Docker,
    name: &str,
    ipv4_prefix: Option<String>,
    bridge: &str,
) -> Result<()> {
    let ipam_config = IpamConfig {
        subnet: ipv4_prefix,
        ..Default::default()
    };

    let ipam = Ipam {
        driver: Some("default".to_string()),
        config: Some(vec![ipam_config]),
        ..Default::default()
    };

    let mut options = HashMap::new();
    options.insert(
        "com.docker.network.bridge.name".to_string(),
        bridge.to_string(),
    );
    let create_request = NetworkCreateRequest {
        name: name.to_owned(),
        driver: Some("bridge".to_owned()),
        options: Some(options),
        ipam: Some(ipam),
        internal: Some(false),
        enable_ipv6: Some(false),
        ..Default::default()
    };

    match docker.create_network(create_request).await {
        Ok(response) => println!("Container network created: {:?}", response),
        Err(e) => eprintln!("Error creating container network: {}", e),
    }

    Ok(())
}
pub async fn delete_network(docker: &Docker, name: &str) -> Result<()> {
    match docker.remove_network(name).await {
        Ok(_) => println!("Container network deleted: {}", name),
        Err(e) => eprintln!("Error deleting container network: {}", e),
    }

    Ok(())
}
pub async fn run_container(
    docker: &Docker,
    name: &str,
    image: &str,
    env_vars: Vec<&str>,
    volumes: Vec<&str>,
    capabilities: Vec<&str>,
    network_attachments: Vec<ContainerNetworkAttachment>,
) -> Result<()> {
    // Environment variables

    let env: Vec<String> = env_vars.iter().map(|s| s.to_string()).collect();

    // Volume bindings
    let binds: Vec<String> = volumes.iter().map(|s| s.to_string()).collect();

    // Capabilities
    let caps: Vec<String> = capabilities.iter().map(|s| s.to_string()).collect();

    // Endpoint config for static IP on sherpa-management network
    let mut endpoints_config = HashMap::new();

    for attachment in network_attachments {
        endpoints_config.insert(
            attachment.name,
            EndpointSettings {
                ipam_config: Some(EndpointIpamConfig {
                    ipv4_address: attachment.ipv4_address,
                    ipv6_address: None,
                    link_local_ips: None,
                }),
                ..Default::default()
            },
        );
    }

    let networking_config = NetworkingConfig {
        endpoints_config: Some(endpoints_config),
    };

    let host_config = HostConfig {
        binds: Some(binds),
        cap_add: Some(caps),
        auto_remove: Some(true), // like --rm flag, removes container automatically when stopped
        ..Default::default()
    };

    // Full container config
    let config = ContainerCreateBody {
        image: Some(image.to_string()),
        env: Some(env),
        host_config: Some(host_config),
        networking_config: Some(networking_config),
        tty: Some(true),
        open_stdin: Some(true),
        ..Default::default()
    };

    let create_opts = CreateContainerOptions {
        name: Some(name.to_string()),
        ..Default::default()
    };

    // Create the container
    println!("Creating container: {name}");
    let ContainerCreateResponse { id, .. } = docker
        .create_container(Some(create_opts), config)
        .await
        .with_context(|| format!("Error creating container: {name}"))?;

    // Start the container
    println!("Starting container: {name}");
    docker
        .start_container(&id, None::<StartContainerOptions>)
        .await
        .with_context(|| format!("Error starting container {name}"))?;

    // After starting the container:
    let inspect_options = Some(InspectContainerOptions { size: false });
    let details = docker.inspect_container(&id, inspect_options).await?;

    // Get the status
    if let Some(state) = &details.state {
        println!(
            "Container status: {}",
            if state.status.is_some() {
                state.status.unwrap().to_string()
            } else {
                "unknown".to_string()
            }
        );
        println!(
            "Exit code: {}",
            if state.exit_code.is_some() {
                state.exit_code.unwrap().to_string()
            } else {
                "unknown".to_string()
            }
        );
    }
    Ok(())
}

pub async fn kill_container(docker: &Docker, name: &str) -> Result<()> {
    docker
        .kill_container(
            name,
            Some(KillContainerOptions {
                signal: "SIGKILL".to_string(),
            }),
        )
        .await
        .with_context(|| format!("Error destroying container: {name}"))?;

    println!("Destroyed container: {name}");
    Ok(())
}

pub async fn remove_container(docker: &Docker, name: &str) -> Result<()> {
    // Wait for the container to exit, then remove (emulates --rm)
    docker
        .remove_container(
            name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await?;
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
