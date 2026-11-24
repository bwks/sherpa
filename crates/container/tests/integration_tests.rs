/// Integration tests for the container crate.
///
/// These tests require a running Docker daemon.
/// Run: cargo test -p container -- --ignored --test-threads=1
///
/// Tests use alpine:latest as the base image — pull it first:
///   docker pull alpine:latest
///
/// Tests create and destroy their own containers and networks.
/// Container/network names are prefixed with "sherpa-test-" to avoid collisions.
/// Each test owns a distinct network name, Linux bridge and subnet, and removes them
/// even when it fails, so one failure cannot cascade into the others.
use anyhow::{Result, ensure};
use bollard::Docker;
use bollard::query_parameters::InspectContainerOptions;
use std::time::Duration;

use container::{
    BridgeSubnet, ContainerMemory, ContainerSpec, create_docker_bridge_network, delete_network,
    docker_connection, exec_container_with_retry, get_local_images, kill_container,
    list_container_memory, list_containers, list_networks, load_image, remove_container,
    run_container,
};
use shared::data::ContainerNetworkAttachment;

// ============================================================================
// Constants
// ============================================================================

const TEST_IMAGE: &str = "alpine:latest";

/// A test-owned Docker bridge network.
struct TestNet {
    name: String,
    bridge: String,
    subnet: String,
}

impl TestNet {
    /// `index` must be unique per test: it selects the bridge name and the subnet.
    fn new(suffix: &str, index: u8) -> Self {
        Self {
            name: format!("sherpa-test-net-{suffix}"),
            // Linux interface names are limited to 15 characters.
            bridge: format!("br-sherpa-t{index}"),
            subnet: format!("192.168.{}.0/24", 200 + u16::from(index)),
        }
    }

    fn ipv4(&self) -> BridgeSubnet {
        BridgeSubnet {
            subnet: self.subnet.clone(),
            gateway: None,
        }
    }
}

fn test_network(name: &str) -> ContainerNetworkAttachment {
    ContainerNetworkAttachment {
        name: name.to_string(),
        ipv4_address: None,
        ipv6_address: None,
        linux_interface_name: None,
        admin_down: false,
    }
}

/// Helper: create a test network and verify it exists.
async fn create_test_network(docker: &Docker, net: &TestNet) -> Result<()> {
    create_docker_bridge_network(docker, &net.name, net.ipv4(), None, &net.bridge).await?;

    // Verify network was actually created
    let networks = list_networks(docker).await?;
    ensure!(
        networks
            .iter()
            .any(|n| n.name.as_deref() == Some(net.name.as_str())),
        "Network {} should exist after creation",
        net.name
    );
    Ok(())
}

/// Helper: create a test container attached to a dedicated bridge network.
/// Container runs `sleep 3600` to stay alive for testing.
/// Cleans up any stale leftovers from previous failed runs first.
async fn create_test_container(docker: &Docker, name: &str, net: &TestNet) -> Result<()> {
    cleanup(docker, name, &net.name).await;
    create_test_network(docker, net).await?;

    run_container(
        docker,
        ContainerSpec {
            name: name.to_string(),
            hostname: None,
            image: TEST_IMAGE.to_string(),
            env_vars: vec![],
            volumes: vec![],
            capabilities: vec![],
            management_network: test_network(&net.name),
            additional_networks: vec![],
            commands: vec!["sleep".to_string(), "3600".to_string()],
            privileged: false,
            shm_size: None,
            user: None,
            memory_mib: None,
            cpu_count: None,
        },
    )
    .await?;

    Ok(())
}

/// Helper: clean up a test container and its network.
/// Called both before (to remove stale leftovers) and after each test.
async fn cleanup(docker: &Docker, container_name: &str, net_name: &str) {
    let _ = kill_container(docker, container_name).await;
    let _ = remove_container(docker, container_name).await;
    let _ = delete_network(docker, net_name).await;
}

async fn container_listed(docker: &Docker, name: &str) -> Result<bool> {
    let containers = list_containers(docker).await?;
    Ok(containers.iter().any(|c| {
        c.names
            .as_ref()
            .is_some_and(|n| n.iter().any(|n| n.contains(name)))
    }))
}

// ============================================================================
// Container with env vars and volumes
// ============================================================================

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn container_run_002_the_manifests_environment_variables_are_set_in_the_container()
-> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-env";
    let net = TestNet::new("env", 8);

    let result: Result<()> = async {
        cleanup(&docker, name, &net.name).await;
        create_test_network(&docker, &net).await?;

        run_container(
            &docker,
            ContainerSpec {
                name: name.to_string(),
                hostname: None,
                image: TEST_IMAGE.to_string(),
                env_vars: vec!["MY_VAR=hello".to_string(), "OTHER=world".to_string()],
                volumes: vec![],
                capabilities: vec![],
                management_network: test_network(&net.name),
                additional_networks: vec![],
                commands: vec!["sleep".to_string(), "3600".to_string()],
                privileged: false,
                shm_size: None,
                user: None,
                memory_mib: None,
                cpu_count: None,
            },
        )
        .await?;

        // exec_container doesn't return output, so verify the env vars via inspect.
        let info = docker
            .inspect_container(
                name,
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await?;
        let env = info
            .config
            .as_ref()
            .and_then(|c| c.env.as_ref())
            .expect("should have env");
        ensure!(env.iter().any(|e| e == "MY_VAR=hello"), "MY_VAR missing");
        ensure!(env.iter().any(|e| e == "OTHER=world"), "OTHER missing");
        Ok(())
    }
    .await;
    cleanup(&docker, name, &net.name).await;
    result
}

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn container_run_004_a_privileged_container_runs_privileged() -> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-priv";
    let net = TestNet::new("priv", 9);

    let result: Result<()> = async {
        cleanup(&docker, name, &net.name).await;
        create_test_network(&docker, &net).await?;

        run_container(
            &docker,
            ContainerSpec { name: name.to_string(), hostname: None, image: TEST_IMAGE.to_string(), env_vars: vec![], volumes: vec![], capabilities: vec![], management_network: test_network(&net.name), additional_networks: vec![], commands: vec!["sleep".to_string(), "3600".to_string()], privileged: true, shm_size: // privileged
            None, user: None, memory_mib: None, cpu_count: None },
        )
        .await?;

        let info = docker
            .inspect_container(
                name,
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await?;
        let privileged = info
            .host_config
            .as_ref()
            .and_then(|h| h.privileged)
            .unwrap_or(false);
        ensure!(privileged, "Container should be privileged");
        Ok(())
    }
    .await;
    cleanup(&docker, name, &net.name).await;
    result
}

// ============================================================================
// Failures are reported, and cleanup is idempotent
// ============================================================================

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn management_network_005_a_management_network_docker_cannot_make_is_reported() -> Result<()>
{
    let docker = docker_connection()?;
    let result = create_docker_bridge_network(
        &docker,
        "sherpa-test-net-invalid",
        BridgeSubnet {
            subnet: "not-a-subnet".to_string(),
            gateway: None,
        },
        None,
        "br-sherpa-t90",
    )
    .await;
    let _ = delete_network(&docker, "sherpa-test-net-invalid").await;
    ensure!(result.is_err(), "an invalid subnet must fail");
    Ok(())
}

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn management_network_006_making_a_docker_management_network_again_succeeds() -> Result<()> {
    let docker = docker_connection()?;
    let net = TestNet::new("again", 51);
    let _ = delete_network(&docker, &net.name).await;
    create_test_network(&docker, &net).await?;
    let again =
        create_docker_bridge_network(&docker, &net.name, net.ipv4(), None, &net.bridge).await;
    delete_network(&docker, &net.name).await?;
    again
}

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn link_cleanup_006_a_network_in_use_is_not_removed_and_one_gone_counts_as_removed()
-> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-net-in-use";
    let net = TestNet::new("in-use", 52);
    create_test_container(&docker, name, &net).await?;
    let in_use = delete_network(&docker, &net.name).await;
    cleanup(&docker, name, &net.name).await;
    ensure!(
        in_use.is_err(),
        "a network with a container must not delete"
    );
    delete_network(&docker, &net.name).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn test_remove_missing_container_succeeds() -> Result<()> {
    let docker = docker_connection()?;
    remove_container(&docker, "sherpa-test-container-missing").await
}

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn image_import_014_a_container_archive_that_cannot_be_read_through_is_a_failed_import()
-> Result<()> {
    let docker = docker_connection()?;
    // A directory opens, but reading it fails.
    let directory = std::env::temp_dir();
    let error = load_image(&docker, directory.to_str().unwrap(), |_| {})
        .await
        .unwrap_err();
    ensure!(
        format!("{error:#}").contains("Failed to read image archive"),
        "{error:#}"
    );
    Ok(())
}

/// A container created with limits runs with a hard memory limit, no swap beyond it and a CPU
/// quota, and lists its memory limit for admission.
#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn container_run_008_memory_and_cpu_count_are_a_hard_limit_without_swap_and_a_cpu_quota()
-> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-limits";
    let net = TestNet::new("limits", 55);
    cleanup(&docker, name, &net.name).await;
    create_test_network(&docker, &net).await?;
    let checks: Result<()> = async {
        run_container(
            &docker,
            ContainerSpec {
                name: name.to_string(),
                hostname: None,
                image: TEST_IMAGE.to_string(),
                env_vars: vec![],
                volumes: vec![],
                capabilities: vec![],
                management_network: test_network(&net.name),
                additional_networks: vec![],
                commands: vec!["sleep".to_string(), "3600".to_string()],
                privileged: false,
                shm_size: None,
                user: None,
                memory_mib: Some(256),
                cpu_count: Some(1),
            },
        )
        .await?;
        let details = docker
            .inspect_container(name, None::<InspectContainerOptions>)
            .await?;
        let host_config = details
            .host_config
            .ok_or_else(|| anyhow::anyhow!("no host config for {name}"))?;
        ensure!(
            host_config.memory == Some(268_435_456),
            "wrong memory limit: {:?}",
            host_config.memory
        );
        ensure!(
            host_config.memory_swap == Some(268_435_456),
            "swap is allowed: {:?}",
            host_config.memory_swap
        );
        ensure!(
            host_config.nano_cpus == Some(1_000_000_000),
            "wrong CPU quota: {:?}",
            host_config.nano_cpus
        );
        let listed = list_container_memory(&docker).await?;
        ensure!(
            listed.contains(&ContainerMemory {
                name: name.to_string(),
                active: true,
                limit_bytes: 268_435_456,
            }),
            "the limit is not listed: {listed:?}"
        );
        Ok(())
    }
    .await;
    cleanup(&docker, name, &net.name).await;
    checks
}

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn container_run_012_a_container_that_cannot_join_one_of_its_networks_is_removed()
-> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-unattachable";
    let net = TestNet::new("unattachable", 53);
    cleanup(&docker, name, &net.name).await;
    create_test_network(&docker, &net).await?;
    let result = run_container(
        &docker,
        ContainerSpec {
            name: name.to_string(),
            hostname: None,
            image: TEST_IMAGE.to_string(),
            env_vars: vec![],
            volumes: vec![],
            capabilities: vec![],
            management_network: test_network(&net.name),
            additional_networks: vec![test_network("sherpa-test-net-missing")],
            commands: vec!["sleep".to_string(), "3600".to_string()],
            privileged: false,
            shm_size: None,
            user: None,
            memory_mib: None,
            cpu_count: None,
        },
    )
    .await;
    let left = container_listed(&docker, name).await?;
    cleanup(&docker, name, &net.name).await;
    ensure!(result.is_err(), "attaching a missing network must fail");
    ensure!(!left, "the container that failed to attach must be removed");
    Ok(())
}

// ============================================================================
// Cisco IOL
// ============================================================================

/// The repo docker-cisco-iol builds the `cisco_iol` image as.
const IOL_REPO: &str = "localrepo/cisco_iol";

/// A `localrepo/cisco_iol` image, or `None` without one. The image is built from the user's
/// own CML image, so the IOL tests are skipped where there is none.
async fn iol_image(docker: &Docker) -> Result<Option<String>> {
    Ok(get_local_images(docker, Some(IOL_REPO))
        .await?
        .into_iter()
        .next())
}

/// Whether `address` answers a ping from the host within `attempts` seconds or so.
async fn answers_ping(address: &str, attempts: u32) -> bool {
    for _ in 0..attempts {
        let answered = std::process::Command::new("ping")
            .args(["-c", "1", "-W", "1", address])
            .output()
            .is_ok_and(|output| output.status.success());
        if answered {
            return true;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    false
}

#[tokio::test]
#[ignore = "requires a Docker daemon and a localrepo/cisco_iol image built by docker-cisco-iol"]
async fn container_run_013_a_cisco_iol_container_runs_unprivileged_holds_its_address_and_relays_its_interfaces()
-> Result<()> {
    let docker = docker_connection()?;
    let Some(image) = iol_image(&docker).await? else {
        eprintln!("skipped: no {IOL_REPO} image");
        return Ok(());
    };
    let name = "sherpa-test-iol";
    let net = TestNet::new("iol", 40);
    let data_net = TestNet::new("iol-data", 41);
    cleanup(&docker, name, &net.name).await;
    let _ = delete_network(&docker, &data_net.name).await;
    create_test_network(&docker, &net).await?;
    create_test_network(&docker, &data_net).await?;

    let config_dir = std::env::temp_dir().join(format!("sherpa-test-iol-{}", std::process::id()));
    std::fs::create_dir_all(&config_dir)?;
    std::fs::write(
        config_dir.join("ios_config.txt"),
        "hostname iol-test\n\
         ip domain name test.local\n\
         interface Ethernet0/0\n \
         ip address 192.168.240.10 255.255.255.0\n \
         no shutdown\n\
         !\n\
         line con 0\n \
         logging synchronous\n\
         !\n\
         end\n",
    )?;

    let checks: Result<()> = async {
        let running = run_container(
            &docker,
            ContainerSpec {
                name: name.to_string(),
                hostname: None,
                image: image.clone(),
                env_vars: vec![
                    "IOL_ID=7".to_string(),
                    "IOL_MEMORY_MB=768".to_string(),
                    "IOL_ETH_PORTS=4".to_string(),
                ],
                volumes: vec![format!("{}:/config:ro", config_dir.display())],
                capabilities: vec!["NET_ADMIN".to_string()],
                management_network: ContainerNetworkAttachment {
                    ipv4_address: Some("192.168.240.10".to_string()),
                    ..test_network(&net.name)
                },
                additional_networks: vec![ContainerNetworkAttachment {
                    linux_interface_name: Some("eth1".to_string()),
                    ..test_network(&data_net.name)
                }],
                commands: vec![],
                privileged: false,
                shm_size: None,
                user: None,
                memory_mib: Some(768 + 128),
                cpu_count: Some(1),
            },
        )
        .await?;
        ensure!(running, "{image} is not running");

        // The entrypoint takes Docker's address off eth0, so only IOL answers for it.
        exec_container_with_retry(
            &docker,
            name,
            vec!["sh", "-c", "! ip -4 addr show dev eth0 | grep -q inet"],
            10,
            Duration::from_secs(1),
        )
        .await?;
        exec_container_with_retry(
            &docker,
            name,
            vec![
                "sh",
                "-c",
                "grep -q 'eth0 attached' /tmp/iol-bridge.log && \
                 grep -q 'eth1 attached' /tmp/iol-bridge.log",
            ],
            10,
            Duration::from_secs(1),
        )
        .await?;
        ensure!(
            answers_ping("192.168.240.10", 240).await,
            "IOL did not answer on its management address"
        );
        Ok(())
    }
    .await;
    cleanup(&docker, name, &net.name).await;
    let _ = delete_network(&docker, &data_net.name).await;
    let _ = std::fs::remove_dir_all(&config_dir);
    checks
}
