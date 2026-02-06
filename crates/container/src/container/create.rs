use std::collections::HashMap;

use anyhow::{Context, Result};
use bollard::Docker;
use bollard::models::{
    ContainerCreateBody, ContainerCreateResponse, EndpointIpamConfig, EndpointSettings, HostConfig,
    NetworkConnectRequest, NetworkingConfig,
};
use bollard::query_parameters::{
    CreateContainerOptions, InspectContainerOptions, StartContainerOptions,
};

use shared::data::ContainerNetworkAttachment;

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
