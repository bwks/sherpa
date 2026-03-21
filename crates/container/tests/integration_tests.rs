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
use anyhow::Result;
use bollard::Docker;

use container::{
    create_docker_bridge_network, delete_network, docker_connection, exec_container,
    exec_container_detached, exec_container_with_retry, get_local_images, kill_container,
    list_containers, list_networks, pause_container, remove_container, run_container,
    start_container, stop_container, unpause_container,
};
use shared::data::ContainerNetworkAttachment;

// ============================================================================
// Constants
// ============================================================================

const TEST_IMAGE: &str = "alpine:latest";

fn test_network(name: &str) -> ContainerNetworkAttachment {
    ContainerNetworkAttachment {
        name: name.to_string(),
        ipv4_address: None,
        ipv6_address: None,
        linux_interface_name: None,
        admin_down: false,
    }
}

/// Helper: create a test container attached to a dedicated bridge network.
/// Container runs `sleep 3600` to stay alive for testing.
/// Cleans up any stale leftovers from previous failed runs first.
async fn create_test_container(docker: &Docker, name: &str, net_name: &str) -> Result<()> {
    cleanup(docker, name, net_name).await;

    create_docker_bridge_network(
        docker,
        net_name,
        Some("192.168.200.0/24".to_string()),
        None,
        &format!("br-{}", &net_name[..std::cmp::min(net_name.len(), 12)]),
    )
    .await?;

    // Verify network was actually created (create_docker_bridge_network swallows errors)
    let networks = list_networks(docker).await?;
    assert!(
        networks.iter().any(|n| n.name.as_deref() == Some(net_name)),
        "Network {} should exist after creation",
        net_name
    );

    run_container(
        docker,
        name,
        TEST_IMAGE,
        vec![],
        vec![],
        vec![],
        test_network(net_name),
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
    let net = "sherpa-test-net-run-remove";

    create_test_container(&docker, name, net).await?;

    // Verify container appears in list
    let containers = list_containers(&docker).await?;
    let found = containers.iter().any(|c| {
        c.names
            .as_ref()
            .is_some_and(|n| n.iter().any(|n| n.contains(name)))
    });
    assert!(found, "Container should appear in list");

    // Kill and remove
    kill_container(&docker, name).await?;
    remove_container(&docker, name).await?;

    // Verify gone
    let containers = list_containers(&docker).await?;
    let found = containers.iter().any(|c| {
        c.names
            .as_ref()
            .is_some_and(|n| n.iter().any(|n| n.contains(name)))
    });
    assert!(!found, "Container should be gone after removal");

    delete_network(&docker, net).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_stop_and_start_container() -> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-stop-start";
    let net = "sherpa-test-net-stop-start";

    create_test_container(&docker, name, net).await?;

    // Stop
    stop_container(&docker, name).await?;

    // Start again
    start_container(&docker, name).await?;

    // Verify running
    let info = docker
        .inspect_container(name, None::<bollard::query_parameters::InspectContainerOptions>)
        .await?;
    let running = info.state.as_ref().and_then(|s| s.running).unwrap_or(false);
    assert!(running, "Container should be running after start");

    cleanup(&docker, name, net).await;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_pause_and_unpause_container() -> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-pause";
    let net = "sherpa-test-net-pause";

    create_test_container(&docker, name, net).await?;

    // Pause
    pause_container(&docker, name).await?;
    let info = docker
        .inspect_container(name, None::<bollard::query_parameters::InspectContainerOptions>)
        .await?;
    let paused = info.state.as_ref().and_then(|s| s.paused).unwrap_or(false);
    assert!(paused, "Container should be paused");

    // Unpause
    unpause_container(&docker, name).await?;
    let info = docker
        .inspect_container(name, None::<bollard::query_parameters::InspectContainerOptions>)
        .await?;
    let paused = info.state.as_ref().and_then(|s| s.paused).unwrap_or(false);
    assert!(!paused, "Container should not be paused after unpause");

    cleanup(&docker, name, net).await;
    Ok(())
}

// ============================================================================
// Container exec
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_exec_container_success() -> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-exec";
    let net = "sherpa-test-net-exec";

    create_test_container(&docker, name, net).await?;

    // Run a command that succeeds
    exec_container(&docker, name, vec!["echo", "hello"]).await?;

    cleanup(&docker, name, net).await;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_exec_container_failure_returns_error() -> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-exec-fail";
    let net = "sherpa-test-net-exec-fail";

    create_test_container(&docker, name, net).await?;

    // Run a command that fails (exit code != 0)
    let result = exec_container(&docker, name, vec!["false"]).await;
    assert!(result.is_err(), "Non-zero exit should produce error");

    cleanup(&docker, name, net).await;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_exec_container_detached() -> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-exec-detach";
    let net = "sherpa-test-net-exec-detach";

    create_test_container(&docker, name, net).await?;

    // Detached exec should return immediately
    exec_container_detached(&docker, name, vec!["sleep", "10"]).await?;

    cleanup(&docker, name, net).await;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_exec_container_with_retry_succeeds() -> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-exec-retry";
    let net = "sherpa-test-net-exec-retry";

    create_test_container(&docker, name, net).await?;

    exec_container_with_retry(
        &docker,
        name,
        vec!["echo", "ok"],
        3,
        std::time::Duration::from_millis(100),
    )
    .await?;

    cleanup(&docker, name, net).await;
    Ok(())
}

// ============================================================================
// Networks
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_create_and_delete_bridge_network() -> Result<()> {
    let docker = docker_connection()?;
    let net_name = "sherpa-test-net-bridge";

    create_docker_bridge_network(
        &docker,
        net_name,
        Some("192.168.99.0/24".to_string()),
        None,
        "br-sherpa-test",
    )
    .await?;

    // Verify network exists
    let networks = list_networks(&docker).await?;
    let found = networks.iter().any(|n| n.name.as_deref() == Some(net_name));
    assert!(found, "Network should exist after creation");

    // Delete
    delete_network(&docker, net_name).await?;

    // Verify gone
    let networks = list_networks(&docker).await?;
    let found = networks.iter().any(|n| n.name.as_deref() == Some(net_name));
    assert!(!found, "Network should be gone after deletion");

    Ok(())
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
    use container::pull_image;

    let count = std::sync::atomic::AtomicU32::new(0);
    pull_image("alpine", "latest", |_msg| {
        count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    })
    .await?;

    // Should have received at least one progress message
    assert!(
        count.load(std::sync::atomic::Ordering::Relaxed) > 0,
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
    let net = "sherpa-test-net-env";

    cleanup(&docker, name, net).await;
    create_docker_bridge_network(
        &docker,
        net,
        Some("192.168.201.0/24".to_string()),
        None,
        &format!("br-{}", &net[..12]),
    )
    .await?;

    run_container(
        &docker,
        name,
        TEST_IMAGE,
        vec!["MY_VAR=hello".to_string(), "OTHER=world".to_string()],
        vec![],
        vec![],
        test_network(net),
        vec![],
        vec!["sleep".to_string(), "3600".to_string()],
        false,
        None,
        None,
    )
    .await?;

    // Verify env var is set inside container
    // exec "env" and check output would be ideal, but exec_container doesn't return output.
    // Instead verify the container was created with the env vars via inspect.
    let info = docker
        .inspect_container(name, None::<bollard::query_parameters::InspectContainerOptions>)
        .await?;
    let env = info
        .config
        .as_ref()
        .and_then(|c| c.env.as_ref())
        .expect("should have env");
    assert!(env.iter().any(|e| e == "MY_VAR=hello"));
    assert!(env.iter().any(|e| e == "OTHER=world"));

    cleanup(&docker, name, net).await;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_run_container_privileged() -> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-priv";
    let net = "sherpa-test-net-priv";

    cleanup(&docker, name, net).await;
    create_docker_bridge_network(
        &docker,
        net,
        Some("192.168.202.0/24".to_string()),
        None,
        &format!("br-{}", &net[..12]),
    )
    .await?;

    run_container(
        &docker,
        name,
        TEST_IMAGE,
        vec![],
        vec![],
        vec![],
        test_network(net),
        vec![],
        vec!["sleep".to_string(), "3600".to_string()],
        true, // privileged
        None,
        None,
    )
    .await?;

    let info = docker
        .inspect_container(name, None::<bollard::query_parameters::InspectContainerOptions>)
        .await?;
    let privileged = info
        .host_config
        .as_ref()
        .and_then(|h| h.privileged)
        .unwrap_or(false);
    assert!(privileged, "Container should be privileged");

    cleanup(&docker, name, net).await;
    Ok(())
}
