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
use shared::konst::{CONTAINER_VM_RUNTIME, DOCKER_ENDPOINT_IFNAME_OPT};

use crate::error::ContainerError;

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
/// Docker's name for every capability.
const ALL_CAPABILITIES: &str = "ALL";
/// The security options that, with every capability and no masked or read-only paths in
/// `/proc`, give a VM-run container what `--privileged` gives a runc one, except the host's
/// devices.
const VM_SECURITY_OPTS: &[&str] = &["apparmor=unconfined", "seccomp=unconfined"];
/// The command a network namespace holder runs: nothing, for as long as it lives.
const NETNS_HOLDER_ENTRYPOINT: &[&str] = &["sleep", "infinity"];

/// Where a container's processes run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ContainerIsolation {
    #[default]
    Host,
    Vm {
        netns_holder: String,
    },
}

/// A container for `run_container` to create and start.
#[derive(Clone, Debug)]
pub struct ContainerSpec {
    pub name: String,
    pub hostname: Option<String>,
    pub image: String,
    pub env_vars: Vec<String>,
    pub volumes: Vec<String>,
    pub capabilities: Vec<String>,
    pub management_network: ContainerNetworkAttachment,
    pub additional_networks: Vec<ContainerNetworkAttachment>,
    pub commands: Vec<String>,
    pub privileged: bool,
    pub shm_size: Option<i64>,
    pub user: Option<String>,
    pub memory_mib: Option<u64>,
    pub cpu_count: Option<u8>,
    pub isolation: ContainerIsolation,
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
    let name = spec.name.clone();
    let additional_networks = vm_data_networks(&spec)?;
    let config = create_body(spec)
        .with_context(|| format!("Invalid resource limits for container {name}"))?;

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

/// The data networks `spec`'s container is attached to after it starts. A VM reads its
/// interfaces only as it boots, so a VM-run container is attached to none: its disabled ones,
/// which would have no carrier in it anyway, are left out, and an enabled one is refused.
fn vm_data_networks(spec: &ContainerSpec) -> Result<Vec<ContainerNetworkAttachment>> {
    if spec.isolation == ContainerIsolation::Host {
        return Ok(spec.additional_networks.clone());
    }
    if spec
        .additional_networks
        .iter()
        .any(|network| !network.admin_down)
    {
        return Err(ContainerError::VmDataNetworks {
            name: spec.name.clone(),
        }
        .into());
    }
    if !spec.additional_networks.is_empty() {
        tracing::debug!(container_name = %spec.name, "Disabled data networks left out of a VM-run container");
    }
    Ok(vec![])
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

/// The body Docker creates the container from. Only the management network is attached at
/// creation, so it is the first interface.
fn create_body(spec: ContainerSpec) -> Result<ContainerCreateBody> {
    let ContainerSpec {
        name: _,
        hostname,
        image,
        env_vars,
        volumes,
        capabilities,
        management_network,
        additional_networks: _,
        commands,
        privileged,
        shm_size,
        user,
        memory_mib,
        cpu_count,
        isolation,
    } = spec;

    if let ContainerIsolation::Vm { netns_holder } = isolation {
        return Ok(ContainerCreateBody {
            image: Some(image),
            env: Some(env_vars),
            host_config: Some(HostConfig {
                binds: Some(volumes),
                cap_add: Some(vec![ALL_CAPABILITIES.to_string()]),
                security_opt: Some(VM_SECURITY_OPTS.iter().map(ToString::to_string).collect()),
                masked_paths: Some(vec![]),
                readonly_paths: Some(vec![]),
                auto_remove: Some(false),
                privileged: Some(false),
                runtime: Some(CONTAINER_VM_RUNTIME.to_string()),
                network_mode: Some(format!("container:{netns_holder}")),
                shm_size,
                ..resource_limits(memory_mib, cpu_count)?
            }),
            tty: Some(true),
            open_stdin: Some(true),
            cmd: Some(commands),
            user,
            ..Default::default()
        });
    }

    Ok(ContainerCreateBody {
        hostname,
        image: Some(image),
        env: Some(env_vars),
        host_config: Some(HostConfig {
            binds: Some(volumes),
            cap_add: Some(capabilities),
            auto_remove: Some(false),
            privileged: Some(privileged),
            shm_size,
            ..resource_limits(memory_mib, cpu_count)?
        }),
        networking_config: Some(management_networking(management_network)),
        tty: Some(true),
        open_stdin: Some(true),
        cmd: Some(commands),
        user,
        ..Default::default()
    })
}

/// The networking config that attaches only the management network, so it is the container's
/// first interface.
fn management_networking(management_network: ContainerNetworkAttachment) -> NetworkingConfig {
    let endpoints_config = HashMap::from([(
        management_network.name,
        EndpointSettings {
            ipam_config: Some(EndpointIpamConfig {
                ipv4_address: management_network.ipv4_address,
                ipv6_address: management_network.ipv6_address,
                link_local_ips: None,
            }),
            ..Default::default()
        },
    )]);
    NetworkingConfig {
        endpoints_config: Some(endpoints_config),
    }
}

/// Create and start the runc container whose network namespace the VM-run container of `spec`
/// joins. It holds the management address and the hostname, and runs nothing, so the data
/// interfaces can be made in its namespace before the VM starts and reads them. A holder that
/// cannot be started is removed again.
///
/// # Errors
///
/// Fails when `spec` is not VM-run, or Docker cannot create or start the holder.
#[instrument(skip(docker, spec), fields(container_name = %spec.name), level = "debug")]
pub async fn start_netns_holder(docker: &Docker, spec: &ContainerSpec) -> Result<()> {
    let ContainerIsolation::Vm { netns_holder } = &spec.isolation else {
        return Err(ContainerError::NotVmRun {
            name: spec.name.clone(),
        }
        .into());
    };
    let create_opts = CreateContainerOptions {
        name: Some(netns_holder.clone()),
        ..Default::default()
    };
    tracing::info!(container_name = %netns_holder, "Creating network namespace holder");
    let ContainerCreateResponse { id, .. } = docker
        .create_container(Some(create_opts), netns_holder_body(spec))
        .await
        .with_context(|| format!("Error creating container: {netns_holder}"))?;
    if let Err(error) = docker
        .start_container(&id, None::<StartContainerOptions>)
        .await
    {
        if let Err(removal) = remove_container(docker, &id).await {
            tracing::warn!(container_name = %netns_holder, error = %removal, "Failed to remove a holder that could not be started");
        }
        return Err(error).with_context(|| format!("Error starting container {netns_holder}"));
    }
    Ok(())
}

/// The body of the runc container that holds a VM-run container's network namespace: the
/// node's image, sleeping, with no capabilities, on the management network.
fn netns_holder_body(spec: &ContainerSpec) -> ContainerCreateBody {
    ContainerCreateBody {
        hostname: spec.hostname.clone(),
        image: Some(spec.image.clone()),
        entrypoint: Some(
            NETNS_HOLDER_ENTRYPOINT
                .iter()
                .map(ToString::to_string)
                .collect(),
        ),
        host_config: Some(HostConfig {
            cap_drop: Some(vec![ALL_CAPABILITIES.to_string()]),
            auto_remove: Some(false),
            ..Default::default()
        }),
        networking_config: Some(management_networking(spec.management_network.clone())),
        ..Default::default()
    }
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

    fn vm_spec(additional_networks: Vec<ContainerNetworkAttachment>) -> ContainerSpec {
        ContainerSpec {
            name: "r1-lab".to_string(),
            hostname: Some("r1".to_string()),
            image: "image:1".to_string(),
            env_vars: vec![],
            volumes: vec![],
            capabilities: vec!["NET_ADMIN".to_string()],
            management_network: attachment(None, false),
            additional_networks,
            commands: vec![],
            privileged: true,
            shm_size: None,
            user: None,
            memory_mib: Some(4096),
            cpu_count: Some(2),
            isolation: ContainerIsolation::Vm {
                netns_holder: "r1-netns-lab".to_string(),
            },
        }
    }

    #[test]
    fn container_vm_002_a_vm_run_container_is_unprivileged_with_every_capability_on_kata() {
        let body = create_body(vm_spec(vec![])).unwrap();
        let host = body.host_config.unwrap();
        assert_eq!(host.runtime.as_deref(), Some(CONTAINER_VM_RUNTIME));
        assert_eq!(host.privileged, Some(false));
        assert_eq!(host.cap_add, Some(vec!["ALL".to_string()]));
        assert_eq!(
            host.security_opt,
            Some(vec![
                "apparmor=unconfined".to_string(),
                "seccomp=unconfined".to_string(),
            ])
        );
        assert_eq!(host.masked_paths, Some(vec![]));
        assert_eq!(host.readonly_paths, Some(vec![]));
        assert_eq!(host.network_mode.as_deref(), Some("container:r1-netns-lab"));
        assert_eq!(host.memory, Some(4096 * 1024 * 1024));
        assert!(body.hostname.is_none());
        assert!(body.networking_config.is_none());
    }

    #[test]
    fn container_vm_008_a_vm_run_container_leaves_out_its_disabled_data_networks() {
        let spec = vm_spec(vec![attachment(Some("eth2"), true)]);
        assert!(vm_data_networks(&spec).unwrap().is_empty());
    }

    #[tokio::test]
    async fn container_vm_003_a_vm_run_container_is_refused_docker_data_networks() {
        // Nothing listens here: the refusal comes before any request.
        let docker =
            Docker::connect_with_http("http://127.0.0.1:9", 1, bollard::API_DEFAULT_VERSION)
                .unwrap();
        let error = run_container(&docker, vm_spec(vec![attachment(Some("eth1"), false)]))
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ContainerError>(),
            Some(ContainerError::VmDataNetworks { name }) if name == "r1-lab"
        ));
    }

    #[test]
    fn node_interfaces_013_a_renamed_container_interface_is_named_by_docker_with_its_address() {
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
    fn node_interfaces_011_an_interface_name_reaches_the_rename_command_as_an_argument() {
        let name = "e1;touch /x";
        let command = data_interface_command(&attachment(Some(name), false)).unwrap();
        assert_eq!(command.last().map(String::as_str), Some(name));
        assert!(command.iter().rev().skip(1).all(|arg| !arg.contains(name)));
    }

    #[test]
    fn node_interfaces_014_only_an_active_renamed_container_interface_runs_a_command() {
        assert!(data_interface_command(&attachment(Some("e1-1"), false)).is_some());
        // Disabled interfaces run nothing, so images without `ip` start.
        assert_eq!(
            data_interface_command(&attachment(Some("eth1"), true)),
            None
        );
        assert_eq!(data_interface_command(&attachment(None, false)), None);
    }
}
