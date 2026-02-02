use std::collections::HashMap;
use std::process::Command;

use anyhow::{Context, Result};
use async_compression::Level;
use async_compression::tokio::write::GzipEncoder;
use bollard::Docker;
use bollard::models::{
    ContainerCreateBody, ContainerCreateResponse, ContainerSummary, EndpointIpamConfig,
    EndpointSettings, HostConfig, Ipam, IpamConfig, Network, NetworkConnectRequest,
    NetworkCreateRequest, NetworkingConfig,
};
use bollard::query_parameters::{
    CreateContainerOptions, CreateImageOptionsBuilder, InspectContainerOptions,
    KillContainerOptions, ListContainersOptions, ListImagesOptions, ListNetworksOptions,
    RemoveContainerOptions, StartContainerOptions,
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
        println!("{image}")
    }

    Ok(())
}

pub async fn list_networks(docker_conn: &Docker) -> Result<Vec<Network>> {
    let options = Some(ListNetworksOptions {
        ..Default::default()
    });
    Ok(docker_conn.list_networks(options).await?)
}
pub async fn create_docker_bridge_network(
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

/// Create a Docker macvlan network that uses an existing Linux bridge.
/// This allows Docker containers to attach to pre-created bridges used for VM-VM or VM-Container links.
///
/// # Arguments
/// * `docker` - Docker connection
/// * `parent_interface` - Name of the pre-existing Linux bridge (e.g., "bra0-12345")
/// * `network_name` - Name for the Docker network (e.g., "sherpa-link-a0-12345")
///
/// # Notes
/// * Uses macvlan driver which provides pure L2 connectivity without requiring IP addressing
/// * No IPAM configuration - containers get no IPs on this network
/// * Relies on the bridge already being created via rtnetlink
pub async fn create_docker_macvlan_network(
    docker: &Docker,
    parent_interface: &str,
    network_name: &str,
) -> Result<()> {
    let mut options = HashMap::new();
    options.insert("parent".to_string(), parent_interface.to_string());

    let create_request = NetworkCreateRequest {
        name: network_name.to_owned(),
        driver: Some("macvlan".to_owned()),
        options: Some(options),
        internal: Some(false),
        ipam: None,
        enable_ipv4: Some(false),
        enable_ipv6: Some(false),
        ..Default::default()
    };

    docker
        .create_network(create_request)
        .await
        .with_context(|| {
            format!(
                "Failed to create Docker macvlan network '{}' on bridge '{}'",
                network_name, parent_interface
            )
        })?;

    println!(
        "Created Docker macvlan network '{}' on bridge '{}'",
        network_name, parent_interface
    );

    Ok(())
}

pub async fn delete_network(docker: &Docker, name: &str) -> Result<()> {
    match docker.remove_network(name).await {
        Ok(_) => println!("Destroyed container network: {}", name),
        Err(e) => eprintln!("Error deleting container network: {}", e),
    }

    Ok(())
}
pub async fn run_container(
    docker: &Docker,
    name: &str,
    image: &str,
    env_vars: Vec<String>,
    volumes: Vec<String>,
    capabilities: Vec<&str>,
    management_network: ContainerNetworkAttachment,
    additional_networks: Vec<ContainerNetworkAttachment>,
    commands: Vec<String>,
    privileged: bool,
) -> Result<()> {
    // Environment variables

    // let env = env_vars.iter().map(|s| s.to_string()).collect();

    // Volume bindings
    // let binds = volumes;

    // Capabilities
    let caps = capabilities.iter().map(|s| s.to_string()).collect();

    // Commands
    let cmds = commands.iter().map(|s| s.to_string()).collect();

    // Create container with only the first network (management network)
    // This is attahced first to ensure ordering.
    let mut endpoints_config = HashMap::new();
    endpoints_config.insert(
        management_network.name.clone(),
        EndpointSettings {
            ipam_config: Some(EndpointIpamConfig {
                ipv4_address: management_network.ipv4_address.clone(),
                ipv6_address: None,
                link_local_ips: None,
            }),
            ..Default::default()
        },
    );

    let networking_config = NetworkingConfig {
        endpoints_config: Some(endpoints_config),
    };

    let host_config = HostConfig {
        binds: Some(volumes),
        cap_add: Some(caps),
        auto_remove: Some(true),
        privileged: Some(privileged),
        ..Default::default()
    };

    // Full container config
    let config = ContainerCreateBody {
        image: Some(image.to_string()),
        env: Some(env_vars),
        host_config: Some(host_config),
        networking_config: Some(networking_config),
        tty: Some(true),
        open_stdin: Some(true),
        cmd: Some(cmds), // Add the command here
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

    // Attach remaining networks sequentially to preserve interface order
    // Skip the first network since it was attached during container creation
    for attachment in additional_networks.iter() {
        let connect_request = NetworkConnectRequest {
            container: Some(id.clone()),
            endpoint_config: Some(EndpointSettings {
                ipam_config: Some(EndpointIpamConfig {
                    ipv4_address: attachment.ipv4_address.clone(),
                    ipv6_address: None,
                    link_local_ips: None,
                }),
                ..Default::default()
            }),
        };

        docker
            .connect_network(&attachment.name, connect_request)
            .await
            .with_context(|| {
                format!(
                    "Error connecting network {} to container {name}",
                    attachment.name
                )
            })?;

        println!("Connected network {} to container: {name}", attachment.name);
    }

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
