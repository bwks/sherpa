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
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use container::{
    create_docker_bridge_network, delete_network, docker_connection, exec_container,
    exec_container_detached, exec_container_with_retry, get_local_images, kill_container,
    list_containers, list_networks, pause_container, pull_image, remove_container, run_container,
    start_container, stop_container, unpause_container,
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
    create_docker_bridge_network(
        docker,
        &net.name,
        Some(net.subnet.clone()),
        None,
        &net.bridge,
    )
    .await?;

    // Verify network was actually created (create_docker_bridge_network swallows errors)
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
        name,
        TEST_IMAGE,
        vec![],
        vec![],
        vec![],
        test_network(&net.name),
        vec![],
        vec!["sleep".to_string(), "3600".to_string()],
        false,
        None,
        None,
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
// Connection
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_docker_connection() -> Result<()> {
    let docker = docker_connection()?;
    // Verify connection works by pinging Docker
    docker.ping().await?;
    Ok(())
}

// ============================================================================
// Container lifecycle
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_run_and_remove_container() -> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-run-remove";
    let net = TestNet::new("run-remove", 0);

    let result: Result<()> = async {
        create_test_container(&docker, name, &net).await?;
        ensure!(
            container_listed(&docker, name).await?,
            "Container should appear in list"
        );

        kill_container(&docker, name).await?;
        remove_container(&docker, name).await?;
        ensure!(
            !container_listed(&docker, name).await?,
            "Container should be gone after removal"
        );

        delete_network(&docker, &net.name).await?;
        Ok(())
    }
    .await;
    cleanup(&docker, name, &net.name).await;
    result
}

#[tokio::test]
#[ignore]
async fn test_stop_and_start_container() -> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-stop-start";
    let net = TestNet::new("stop-start", 1);

    let result: Result<()> = async {
        create_test_container(&docker, name, &net).await?;
        stop_container(&docker, name).await?;
        start_container(&docker, name).await?;

        let info = docker
            .inspect_container(
                name,
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await?;
        let running = info.state.as_ref().and_then(|s| s.running).unwrap_or(false);
        ensure!(running, "Container should be running after start");
        Ok(())
    }
    .await;
    cleanup(&docker, name, &net.name).await;
    result
}

#[tokio::test]
#[ignore]
async fn test_pause_and_unpause_container() -> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-pause";
    let net = TestNet::new("pause", 2);

    let result: Result<()> = async {
        create_test_container(&docker, name, &net).await?;

        pause_container(&docker, name).await?;
        let info = docker
            .inspect_container(
                name,
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await?;
        let paused = info.state.as_ref().and_then(|s| s.paused).unwrap_or(false);
        ensure!(paused, "Container should be paused");

        unpause_container(&docker, name).await?;
        let info = docker
            .inspect_container(
                name,
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await?;
        let paused = info.state.as_ref().and_then(|s| s.paused).unwrap_or(false);
        ensure!(!paused, "Container should not be paused after unpause");
        Ok(())
    }
    .await;
    cleanup(&docker, name, &net.name).await;
    result
}

// ============================================================================
// Container exec
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_exec_container_success() -> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-exec";
    let net = TestNet::new("exec", 3);

    let result: Result<()> = async {
        create_test_container(&docker, name, &net).await?;
        exec_container(&docker, name, vec!["echo", "hello"]).await?;
        Ok(())
    }
    .await;
    cleanup(&docker, name, &net.name).await;
    result
}

#[tokio::test]
#[ignore]
async fn test_exec_container_failure_returns_error() -> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-exec-fail";
    let net = TestNet::new("exec-fail", 4);

    let result: Result<()> = async {
        create_test_container(&docker, name, &net).await?;
        let exec = exec_container(&docker, name, vec!["false"]).await;
        ensure!(exec.is_err(), "Non-zero exit should produce error");
        Ok(())
    }
    .await;
    cleanup(&docker, name, &net.name).await;
    result
}

#[tokio::test]
#[ignore]
async fn test_exec_container_detached() -> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-exec-detach";
    let net = TestNet::new("exec-detach", 5);

    let result: Result<()> = async {
        create_test_container(&docker, name, &net).await?;
        // Detached exec should return immediately
        exec_container_detached(&docker, name, vec!["sleep", "10"]).await?;
        Ok(())
    }
    .await;
    cleanup(&docker, name, &net.name).await;
    result
}

#[tokio::test]
#[ignore]
async fn test_exec_container_with_retry_succeeds() -> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-exec-retry";
    let net = TestNet::new("exec-retry", 6);

    let result: Result<()> = async {
        create_test_container(&docker, name, &net).await?;
        exec_container_with_retry(
            &docker,
            name,
            vec!["echo", "ok"],
            3,
            Duration::from_millis(100),
        )
        .await?;
        Ok(())
    }
    .await;
    cleanup(&docker, name, &net.name).await;
    result
}

// ============================================================================
// Networks
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_create_and_delete_bridge_network() -> Result<()> {
    let docker = docker_connection()?;
    let net = TestNet::new("bridge", 7);

    let result: Result<()> = async {
        let _ = delete_network(&docker, &net.name).await;
        create_test_network(&docker, &net).await?;

        delete_network(&docker, &net.name).await?;
        let networks = list_networks(&docker).await?;
        let found = networks
            .iter()
            .any(|n| n.name.as_deref() == Some(net.name.as_str()));
        ensure!(!found, "Network should be gone after deletion");
        Ok(())
    }
    .await;
    let _ = delete_network(&docker, &net.name).await;
    result
}

#[tokio::test]
#[ignore]
async fn test_list_networks_includes_default() -> Result<()> {
    let docker = docker_connection()?;
    let networks = list_networks(&docker).await?;

    // Docker always has a "bridge" network
    let has_bridge = networks.iter().any(|n| n.name.as_deref() == Some("bridge"));
    assert!(has_bridge, "Default bridge network should always exist");

    Ok(())
}

// ============================================================================
// Images
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_get_local_images() -> Result<()> {
    let docker = docker_connection()?;
    let images = get_local_images(&docker).await?;

    // We pulled alpine:latest earlier, it should be in the list
    assert!(
        images.iter().any(|i| i.contains("alpine")),
        "alpine should be in local images, got: {:?}",
        images
    );

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_pull_image() -> Result<()> {
    let count = AtomicU32::new(0);
    pull_image("alpine", "latest", |_msg| {
        count.fetch_add(1, Ordering::Relaxed);
    })
    .await?;

    // Should have received at least one progress message
    assert!(
        count.load(Ordering::Relaxed) > 0,
        "Should receive progress messages during pull"
    );

    Ok(())
}

// ============================================================================
// Container with env vars and volumes
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_run_container_with_env_vars() -> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-env";
    let net = TestNet::new("env", 8);

    let result: Result<()> = async {
        cleanup(&docker, name, &net.name).await;
        create_test_network(&docker, &net).await?;

        run_container(
            &docker,
            name,
            TEST_IMAGE,
            vec!["MY_VAR=hello".to_string(), "OTHER=world".to_string()],
            vec![],
            vec![],
            test_network(&net.name),
            vec![],
            vec!["sleep".to_string(), "3600".to_string()],
            false,
            None,
            None,
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
#[ignore]
async fn test_run_container_privileged() -> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-priv";
    let net = TestNet::new("priv", 9);

    let result: Result<()> = async {
        cleanup(&docker, name, &net.name).await;
        create_test_network(&docker, &net).await?;

        run_container(
            &docker,
            name,
            TEST_IMAGE,
            vec![],
            vec![],
            vec![],
            test_network(&net.name),
            vec![],
            vec!["sleep".to_string(), "3600".to_string()],
            true, // privileged
            None,
            None,
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
