use std::collections::HashMap;

use anyhow::{Context, Result};
use bollard::Docker;
use bollard::models::{
    ContainerCreateBody, ContainerCreateResponse, ContainerStateStatusEnum, EndpointIpamConfig,
    EndpointSettings, HostConfig, NetworkConnectRequest, NetworkingConfig,
};
use bollard::query_parameters::{
    CreateContainerOptions, InspectContainerOptions, StartContainerOptions,
};
use tracing::instrument;

use shared::data::ContainerNetworkAttachment;
use shared::konst::DOCKER_ENDPOINT_IFNAME_OPT;

use super::exec::exec_container_with_retry;

#[allow(clippy::too_many_arguments)]
#[instrument(skip(docker), level = "debug")]
pub async fn run_container(
    docker: &Docker,
    name: &str,
    image: &str,
    env_vars: Vec<String>,
    volumes: Vec<String>,
    capabilities: Vec<String>,
    management_network: ContainerNetworkAttachment,
    additional_networks: Vec<ContainerNetworkAttachment>,
    commands: Vec<String>,
    privileged: bool,
    shm_size: Option<i64>,
    user: Option<String>,
) -> Result<bool> {
    // Environment variables

    // let env = env_vars.iter().map(|s| s.to_string()).collect();

    // Volume bindings
    // let binds = volumes;

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
                ipv6_address: management_network.ipv6_address.clone(),
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
        cap_add: Some(capabilities),
        auto_remove: Some(false),
        privileged: Some(privileged),
        shm_size,
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
        cmd: Some(cmds),
        user,
        ..Default::default()
    };

    let create_opts = CreateContainerOptions {
        name: Some(name.to_string()),
        ..Default::default()
    };

    // Create the container
    tracing::info!(container_name = %name, "Creating container");
    let ContainerCreateResponse { id, .. } = docker
        .create_container(Some(create_opts), config)
        .await
        .with_context(|| format!("Error creating container: {name}"))?;

    // Start the container
    tracing::info!(container_name = %name, "Starting container");
    docker
        .start_container(&id, None::<StartContainerOptions>)
        .await
        .with_context(|| format!("Error starting container {name}"))?;

    // Attach remaining networks sequentially to preserve interface order
    // Skip the first network since it was attached during container creation
    for attachment in &additional_networks {
        let connect_request = NetworkConnectRequest {
            container: Some(id.clone()),
            endpoint_config: Some(data_endpoint_settings(attachment)),
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

        tracing::debug!(container_name = %name, network = %attachment.name, "Connected network to container");

        if let Some(command) = data_interface_command(attachment) {
            exec_container_with_retry(
                docker,
                name,
                vec!["sh", "-c", &command],
                3,
                std::time::Duration::from_secs(2),
            )
            .await
            .with_context(|| {
                format!(
                    "Failed to set up interface {} in container {name}",
                    attachment.name
                )
            })?;
        }
    }

    // After starting the container:
    let inspect_options = Some(InspectContainerOptions { size: false });
    let details = docker.inspect_container(&id, inspect_options).await?;

    // Check if the container is running
    let is_running = if let Some(state) = &details.state {
        let status = state
            .status
            .as_ref()
            .map(|s| format!("{:?}", s))
            .unwrap_or_else(|| "unknown".to_string());
        let exit_code = state.exit_code.unwrap_or(-1);
        tracing::debug!(
            container_name = %name,
            status = %status,
            exit_code = exit_code,
            "Container status after creation"
        );
        state.status == Some(ContainerStateStatusEnum::RUNNING)
    } else {
        false
    };

    Ok(is_running)
}

/// Docker endpoint settings for a data network. Docker names the interface in the container, so
/// no command has to run in the image to rename it.
fn data_endpoint_settings(attachment: &ContainerNetworkAttachment) -> EndpointSettings {
    let mut driver_opts = None;
    if let Some(name) = &attachment.linux_interface_name {
        driver_opts = Some(HashMap::from([(
            DOCKER_ENDPOINT_IFNAME_OPT.to_string(),
            name.clone(),
        )]));
    }
    EndpointSettings {
        ipam_config: Some(EndpointIpamConfig {
            ipv4_address: attachment.ipv4_address.clone(),
            ipv6_address: attachment.ipv6_address.clone(),
            link_local_ips: None,
        }),
        driver_opts,
        ..Default::default()
    }
}

/// The command to run in the container once a data interface is attached, if any: an active,
/// renamed interface is set promiscuous and up. Disabled interfaces need none: the server takes
/// the carrier away from the host, so images without `ip` work.
fn data_interface_command(attachment: &ContainerNetworkAttachment) -> Option<String> {
    if attachment.admin_down {
        return None;
    }
    let name = attachment.linux_interface_name.as_ref()?;
    Some(format!(
        "ip link set {name} promisc on && ip link set {name} up"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(
        linux_interface_name: Option<&str>,
        admin_down: bool,
    ) -> ContainerNetworkAttachment {
        ContainerNetworkAttachment {
            name: "net".to_string(),
            ipv4_address: Some("172.31.0.5".to_string()),
            ipv6_address: None,
            linux_interface_name: linux_interface_name.map(str::to_string),
            admin_down,
        }
    }

    #[test]
    fn docker_names_renamed_interfaces() {
        let settings = data_endpoint_settings(&attachment(Some("e1-1"), false));
        assert_eq!(
            settings
                .driver_opts
                .unwrap()
                .get("com.docker.network.endpoint.ifname"),
            Some(&"e1-1".to_string())
        );
        assert_eq!(
            settings.ipam_config.unwrap().ipv4_address.as_deref(),
            Some("172.31.0.5")
        );
        assert!(
            data_endpoint_settings(&attachment(None, false))
                .driver_opts
                .is_none()
        );
    }

    #[test]
    fn only_active_renamed_interfaces_run_a_command() {
        assert_eq!(
            data_interface_command(&attachment(Some("e1-1"), false)).as_deref(),
            Some("ip link set e1-1 promisc on && ip link set e1-1 up")
        );
        // Disabled interfaces run nothing, so images without `ip` start.
        assert_eq!(
            data_interface_command(&attachment(Some("eth1"), true)),
            None
        );
        assert_eq!(data_interface_command(&attachment(None, false)), None);
    }
}
