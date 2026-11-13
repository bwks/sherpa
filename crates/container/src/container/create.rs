use std::collections::HashMap;
use std::time::Duration;

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

use super::delete::remove_container;
use super::exec::exec_container_with_retry;

/// Sets an attached data interface promiscuous and up. The interface name is the shell's `$1`,
/// so it is never read as script.
const DATA_INTERFACE_SCRIPT: &str = r#"ip link set "$1" promisc on && ip link set "$1" up"#;
/// Retries of the data interface command while the container's network settles.
const DATA_INTERFACE_RETRIES: u32 = 3;
/// The pause between data interface command attempts.
const DATA_INTERFACE_RETRY_DELAY: Duration = Duration::from_secs(2);
/// Bytes in a MiB, the unit of a node's memory.
const BYTES_PER_MIB: u64 = 1024 * 1024;
/// Docker's CPU quota unit, `NanoCpus`, in billionths of a CPU.
const NANO_CPUS_PER_CPU: i64 = 1_000_000_000;

/// A container for `run_container` to create and start.
#[derive(Clone, Debug)]
pub struct ContainerSpec {
    pub name: String,
    pub image: String,
    pub env_vars: Vec<String>,
    /// Bind mounts, as `host:container[:options]`.
    pub volumes: Vec<String>,
    pub capabilities: Vec<String>,
    /// Attached when the container is created, so it is the first interface.
    pub management_network: ContainerNetworkAttachment,
    /// Attached one by one after the container starts, so the interfaces keep this order.
    pub additional_networks: Vec<ContainerNetworkAttachment>,
    pub commands: Vec<String>,
    pub privileged: bool,
    pub shm_size: Option<i64>,
    pub user: Option<String>,
    /// The hard memory limit, in MiB, with no swap. `None` leaves memory unlimited.
    pub memory_mib: Option<u64>,
    /// The CPU quota, in CPUs. `None` leaves CPU unlimited.
    pub cpu_count: Option<u8>,
}

/// Create and start a container, then attach its data networks in order. Returns whether the
/// container is running afterwards.
///
/// A container that is created but cannot be started or attached is removed again, so a retry
/// can create it under the same name.
///
/// # Errors
///
/// Fails when Docker cannot create, start, attach or inspect the container, or a data
/// interface cannot be set up in it.
#[instrument(skip(docker, spec), fields(container_name = %spec.name), level = "debug")]
pub async fn run_container(docker: &Docker, spec: ContainerSpec) -> Result<bool> {
    let ContainerSpec {
        name,
        image,
        env_vars,
        volumes,
        capabilities,
        management_network,
        additional_networks,
        commands,
        privileged,
        shm_size,
        user,
        memory_mib,
        cpu_count,
    } = spec;

    // Only the management network is attached at creation, so it is the first interface.
    let endpoints_config = HashMap::from([(
        management_network.name.clone(),
        EndpointSettings {
            ipam_config: Some(EndpointIpamConfig {
                ipv4_address: management_network.ipv4_address,
                ipv6_address: management_network.ipv6_address,
                link_local_ips: None,
            }),
            ..Default::default()
        },
    )]);

    let config = ContainerCreateBody {
        image: Some(image),
        env: Some(env_vars),
        host_config: Some(HostConfig {
            binds: Some(volumes),
            cap_add: Some(capabilities),
            auto_remove: Some(false),
            privileged: Some(privileged),
            shm_size,
            ..resource_limits(memory_mib, cpu_count)
                .with_context(|| format!("Invalid resource limits for container {name}"))?
        }),
        networking_config: Some(NetworkingConfig {
            endpoints_config: Some(endpoints_config),
        }),
        tty: Some(true),
        open_stdin: Some(true),
        cmd: Some(commands),
        user,
        ..Default::default()
    };

    let create_opts = CreateContainerOptions {
        name: Some(name.clone()),
        ..Default::default()
    };

    tracing::info!(container_name = %name, "Creating container");
    let ContainerCreateResponse { id, .. } = docker
        .create_container(Some(create_opts), config)
        .await
        .with_context(|| format!("Error creating container: {name}"))?;

    match start_and_attach(docker, &id, &name, &additional_networks).await {
        Ok(running) => Ok(running),
        Err(error) => {
            if let Err(removal) = remove_container(docker, &id).await {
                tracing::warn!(container_name = %name, error = %removal, "Failed to remove a container that could not be set up");
            }
            Err(error)
        }
    }
}

/// Start the container `id`, attach its data networks in order and set their interfaces up,
/// then report whether it is running.
async fn start_and_attach(
    docker: &Docker,
    id: &str,
    name: &str,
    additional_networks: &[ContainerNetworkAttachment],
) -> Result<bool> {
    tracing::info!(container_name = %name, "Starting container");
    docker
        .start_container(id, None::<StartContainerOptions>)
        .await
        .with_context(|| format!("Error starting container {name}"))?;

    for attachment in additional_networks {
        let connect_request = NetworkConnectRequest {
            container: Some(id.to_string()),
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
                command.iter().map(String::as_str).collect(),
                DATA_INTERFACE_RETRIES,
                DATA_INTERFACE_RETRY_DELAY,
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

    let details = docker
        .inspect_container(id, Some(InspectContainerOptions { size: false }))
        .await
        .with_context(|| format!("Error inspecting container {name}"))?;
    let Some(state) = &details.state else {
        return Ok(false);
    };
    tracing::debug!(
        container_name = %name,
        status = ?state.status,
        exit_code = ?state.exit_code,
        "Container status after creation"
    );
    Ok(state.status == Some(ContainerStateStatusEnum::RUNNING))
}

/// The host config of a container's limits: `memory_mib` as a hard limit in bytes, with the
/// swap limit equal to it so the container cannot swap, and `cpu_count` as a CPU quota in
/// billionths of a CPU. A limit that is not set stays unlimited, Docker's default.
fn resource_limits(memory_mib: Option<u64>, cpu_count: Option<u8>) -> Result<HostConfig> {
    let memory = memory_mib
        .map(|mib| {
            mib.checked_mul(BYTES_PER_MIB)
                .and_then(|bytes| i64::try_from(bytes).ok())
                .with_context(|| format!("Memory limit of {mib} MiB is too large"))
        })
        .transpose()?;
    Ok(HostConfig {
        memory,
        memory_swap: memory,
        nano_cpus: cpu_count.map(|count| i64::from(count) * NANO_CPUS_PER_CPU),
        ..Default::default()
    })
}

/// Docker endpoint settings for a data network. Docker names the interface in the container, so
/// no command has to run in the image to rename it.
fn data_endpoint_settings(attachment: &ContainerNetworkAttachment) -> EndpointSettings {
    let driver_opts = attachment
        .linux_interface_name
        .as_ref()
        .map(|name| HashMap::from([(DOCKER_ENDPOINT_IFNAME_OPT.to_string(), name.clone())]));
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
fn data_interface_command(attachment: &ContainerNetworkAttachment) -> Option<Vec<String>> {
    if attachment.admin_down {
        return None;
    }
    let name = attachment.linux_interface_name.as_ref()?;
    Some(vec![
        "sh".to_string(),
        "-c".to_string(),
        DATA_INTERFACE_SCRIPT.to_string(),
        "sh".to_string(),
        name.clone(),
    ])
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
    fn limits_are_bytes_without_swap_and_nano_cpus() {
        let limits = resource_limits(Some(512), Some(2)).unwrap();
        assert_eq!(limits.memory, Some(536_870_912));
        assert_eq!(limits.memory_swap, Some(536_870_912));
        assert_eq!(limits.nano_cpus, Some(2_000_000_000));
    }

    #[test]
    fn unset_limits_stay_unlimited() {
        let limits = resource_limits(None, None).unwrap();
        assert_eq!(limits.memory, None);
        assert_eq!(limits.memory_swap, None);
        assert_eq!(limits.nano_cpus, None);
    }

    #[test]
    fn a_memory_limit_too_large_for_docker_is_refused() {
        assert!(resource_limits(Some(u64::MAX / 1024), None).is_err());
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
    fn interface_names_are_shell_arguments_not_script() {
        let name = "e1;touch /x";
        let command = data_interface_command(&attachment(Some(name), false)).unwrap();
        assert_eq!(command.last().map(String::as_str), Some(name));
        assert!(command.iter().rev().skip(1).all(|arg| !arg.contains(name)));
    }

    #[test]
    fn only_active_renamed_interfaces_run_a_command() {
        assert!(data_interface_command(&attachment(Some("e1-1"), false)).is_some());
        // Disabled interfaces run nothing, so images without `ip` start.
        assert_eq!(
            data_interface_command(&attachment(Some("eth1"), true)),
            None
        );
        assert_eq!(data_interface_command(&attachment(None, false)), None);
    }
}
