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
use bollard::query_parameters::{InspectContainerOptions, RemoveImageOptions, TagImageOptions};
use bollard::secret::ContainerStateStatusEnum;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use container::{
    ContainerMemory, ContainerSpec, container_log_tail, container_state,
    create_docker_bridge_network, delete_network, docker_connection, exec_container,
    exec_container_detached, exec_container_with_retry, get_local_images, image_exists,
    kill_container, list_container_memory, list_containers, list_networks, load_image,
    pause_container, pull_image, remove_container, run_container, start_container, stop_container,
    unpause_container,
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
// Connection
// ============================================================================

#[tokio::test]
#[ignore = "requires a Docker daemon"]
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
#[ignore = "requires a Docker daemon"]
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
#[ignore = "requires a Docker daemon"]
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
#[ignore = "requires a Docker daemon"]
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
#[ignore = "requires a Docker daemon"]
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
#[ignore = "requires a Docker daemon"]
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
#[ignore = "requires a Docker daemon"]
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
#[ignore = "requires a Docker daemon"]
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
#[ignore = "requires a Docker daemon"]
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
#[ignore = "requires a Docker daemon"]
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
#[ignore = "requires a Docker daemon"]
async fn test_get_local_images() -> Result<()> {
    let docker = docker_connection()?;
    let images = get_local_images(&docker, None).await?;

    // We pulled alpine:latest earlier, it should be in the list
    assert!(
        images.iter().any(|i| i.contains("alpine")),
        "alpine should be in local images, got: {images:?}"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires a Docker daemon and internet access to pull an image"]
async fn an_official_image_is_found_by_its_full_and_short_name() -> Result<()> {
    let docker = docker_connection()?;
    pull_image("docker.io/library/busybox", "1.36", |_| {}).await?;

    for reference in [
        "docker.io/library/busybox:1.36",
        "library/busybox:1.36",
        "busybox:1.36",
    ] {
        ensure!(
            image_exists(&docker, reference).await?,
            "{reference} was not found"
        );
    }
    ensure!(
        !image_exists(&docker, "docker.io/library/busybox:sherpa-no-such-tag").await?,
        "a tag that was never pulled was found"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires a Docker daemon and internet access to pull an image"]
async fn a_docker_hub_repo_is_listed_by_its_short_and_full_name_only() -> Result<()> {
    let docker = docker_connection()?;
    pull_image("docker.io/library/busybox", "1.36", |_| {}).await?;

    for reference in ["busybox", "docker.io/library/busybox"] {
        let images = get_local_images(&docker, Some(reference)).await?;
        ensure!(
            images.contains(&"busybox:1.36".to_string()),
            "{reference} did not list busybox:1.36: {images:?}"
        );
    }
    for reference in ["docker.io/busybox", "index.docker.io/library/busybox"] {
        let images = get_local_images(&docker, Some(reference)).await?;
        ensure!(images.is_empty(), "{reference} listed {images:?}");
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn a_repo_does_not_list_names_it_is_a_prefix_or_suffix_of() -> Result<()> {
    const REPO: &str = "sherpa-test-scan";
    let docker = docker_connection()?;
    let tags = [REPO, "sherpa-test-scan-box", "sherpa-test/sherpa-test-scan"];
    for repo in tags {
        docker
            .tag_image(
                TEST_IMAGE,
                Some(TagImageOptions {
                    repo: Some(repo.to_string()),
                    tag: Some("1".to_string()),
                }),
            )
            .await?;
    }

    let listed = get_local_images(&docker, Some(REPO)).await;

    for repo in tags {
        docker
            .remove_image(&format!("{repo}:1"), None::<RemoveImageOptions>, None)
            .await?;
    }
    let listed = listed?;
    ensure!(listed == ["sherpa-test-scan:1"], "{REPO} listed {listed:?}");
    Ok(())
}

#[tokio::test]
#[ignore = "requires a Docker daemon"]
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
#[ignore = "requires a Docker daemon"]
async fn test_run_container_with_env_vars() -> Result<()> {
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
async fn test_run_container_privileged() -> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-priv";
    let net = TestNet::new("priv", 9);

    let result: Result<()> = async {
        cleanup(&docker, name, &net.name).await;
        create_test_network(&docker, &net).await?;

        run_container(
            &docker,
            ContainerSpec { name: name.to_string(), image: TEST_IMAGE.to_string(), env_vars: vec![], volumes: vec![], capabilities: vec![], management_network: test_network(&net.name), additional_networks: vec![], commands: vec!["sleep".to_string(), "3600".to_string()], privileged: true, shm_size: // privileged
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
async fn test_create_bridge_network_reports_failure() -> Result<()> {
    let docker = docker_connection()?;
    let result = create_docker_bridge_network(
        &docker,
        "sherpa-test-net-invalid",
        Some("not-a-subnet".to_string()),
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
async fn test_create_existing_bridge_network_succeeds() -> Result<()> {
    let docker = docker_connection()?;
    let net = TestNet::new("again", 51);
    let _ = delete_network(&docker, &net.name).await;
    create_test_network(&docker, &net).await?;
    let again = create_docker_bridge_network(
        &docker,
        &net.name,
        Some(net.subnet.clone()),
        None,
        &net.bridge,
    )
    .await;
    delete_network(&docker, &net.name).await?;
    again
}

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn test_delete_network_in_use_fails_and_missing_succeeds() -> Result<()> {
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
async fn test_load_image_reports_a_read_error() -> Result<()> {
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
async fn a_container_runs_with_its_memory_and_cpu_limits() -> Result<()> {
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
async fn test_run_container_removes_a_container_it_cannot_set_up() -> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-unattachable";
    let net = TestNet::new("unattachable", 53);
    cleanup(&docker, name, &net.name).await;
    create_test_network(&docker, &net).await?;
    let result = run_container(
        &docker,
        ContainerSpec {
            name: name.to_string(),
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

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn a_container_that_exits_at_once_reports_its_exit_code_and_last_log_lines() -> Result<()> {
    let docker = docker_connection()?;
    let name = "sherpa-test-exits";
    let net = TestNet::new("exits", 54);
    cleanup(&docker, name, &net.name).await;
    create_test_network(&docker, &net).await?;
    let checks: Result<()> = async {
        run_container(
            &docker,
            ContainerSpec {
                name: name.to_string(),
                image: TEST_IMAGE.to_string(),
                env_vars: vec![],
                volumes: vec![],
                capabilities: vec![],
                management_network: test_network(&net.name),
                additional_networks: vec![],
                commands: vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    "for i in 1 2 3 4 5; do echo line $i; done; exit 3".to_string(),
                ],
                privileged: false,
                shm_size: None,
                user: None,
                memory_mib: None,
                cpu_count: None,
            },
        )
        .await?;
        let mut state = container_state(&docker, name).await?;
        for _ in 0..50 {
            if state.as_ref().and_then(|state| state.status)
                == Some(ContainerStateStatusEnum::EXITED)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            state = container_state(&docker, name).await?;
        }
        let state = state.ok_or_else(|| anyhow::anyhow!("no state for {name}"))?;
        ensure!(
            state.status == Some(ContainerStateStatusEnum::EXITED),
            "the container did not exit: {state:?}"
        );
        ensure!(state.exit_code == Some(3), "wrong exit code: {state:?}");
        let log = container_log_tail(&docker, name, 2).await?;
        ensure!(log == "line 4\nline 5", "unexpected log tail: {log:?}");
        Ok(())
    }
    .await;
    cleanup(&docker, name, &net.name).await;
    checks
}

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn a_missing_container_has_no_state() -> Result<()> {
    let docker = docker_connection()?;
    let state = container_state(&docker, "sherpa-test-never-created").await?;
    ensure!(
        state.is_none(),
        "a missing container has a state: {state:?}"
    );
    Ok(())
}
