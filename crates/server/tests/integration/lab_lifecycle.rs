use anyhow::{Context, Result, bail, ensure};
use bollard::Docker;
use bollard::query_parameters::{InspectContainerOptions, ListNetworksOptions};
use db::DefaultRequest;
use jato::{Auth, ConnectOptions, Exit, Hop, HostKeyPolicy, Node, Secret, Session};
use reqwest::{Client, StatusCode};
use serde_json::json;
use shared::data::{
    ContainerSettings, DbUser, KindSettings, LabNodeActionResponse, LabPhase, LabState, ModelKind,
    NodeConfig, NodeKind, NodeModel, NodeState, ZtpSettings,
};
use shared::error::RpcErrorCode;
use shared::konst::{SHERPA_LABS_PATH, SSH_PORT};
use sherpad::auth::cookies::AUTH_COOKIE_NAME;
use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::helpers::test_server::{
    TEST_ADMIN_PASSWORD, TEST_DEVICE_PASSWORD, TEST_LAB_ADMIN, TEST_SSH_PUBLIC_KEY, TestServer,
};
use crate::helpers::ws_client::TestWsClient;

/// Helper to create a minimal container-only manifest
fn container_manifest(lab_name: &str) -> serde_json::Value {
    json!({
        "name": lab_name,
        "nodes": [
            {
                "name": "srl1",
                "model": "nokia_srlinux",
            }
        ]
    })
}

/// How long a one-VM lab may take to come up, or to be destroyed.
const VM_TIMEOUT: Duration = Duration::from_mins(5);

/// A lab with one `alpine_linux` VM, `vm1`.
fn vm_manifest(lab_name: &str) -> serde_json::Value {
    json!({
        "name": lab_name,
        "nodes": [
            {
                "name": "vm1",
                "model": "alpine_linux",
            }
        ]
    })
}

/// Helper to bootstrap images by running image.scan
async fn bootstrap_images(ws: &mut TestWsClient, token: &str) -> Result<()> {
    let (_s, response) = ws
        .rpc_call_streaming(
            "image.scan",
            json!({ "token": token }),
            Duration::from_secs(60),
        )
        .await?;
    assert!(
        response.get("error").is_none(),
        "image.scan failed: {response:?}"
    );
    Ok(())
}

/// The ID of container `name` and the image reference it was created from.
async fn container_image(docker: &Docker, name: &str) -> Result<(Option<String>, Option<String>)> {
    let container = docker
        .inspect_container(name, None::<InspectContainerOptions>)
        .await
        .with_context(|| format!("container {name} was not created"))?;
    Ok((
        container.id,
        container.config.and_then(|config| config.image),
    ))
}

/// IPv4-only lab networks for tests that seed lab records directly.
fn ipv4_lab_networks(
    management_network: &str,
    gateway_ipv4: &str,
    router_ipv4: &str,
) -> db::LabNetworks {
    db::LabNetworks {
        management_network: management_network.to_string(),
        gateway_ipv4: gateway_ipv4.to_string(),
        router_ipv4: router_ipv4.to_string(),
        management_network_v6: None,
        loopback_network_v6: None,
        gateway_ipv6: None,
        router_ipv6: None,
    }
}

/// Destroy a lab if it still exists, so a failed test never leaves resources behind.
/// Existence comes from the database, so a refused request cannot look like a missing lab.
/// `token` must belong to the lab owner or an admin.
async fn ensure_lab_destroyed(
    ws: &mut TestWsClient,
    server: &TestServer,
    token: &str,
    lab_id: &str,
) -> Result<()> {
    if db::get_lab(&server.db, lab_id).await.is_err() {
        return Ok(());
    }
    let (_s, response) = ws
        .rpc_call_streaming(
            "destroy",
            json!({ "token": token, "lab_id": lab_id }),
            Duration::from_secs(300),
        )
        .await?;
    ensure!(
        response.get("error").is_none(),
        "Lab {lab_id} cleanup failed: {response}"
    );
    Ok(())
}

/// Clean up after a test. With `SHERPA_TEST_KEEP_FAILED_LABS=1`, a failed test keeps its
/// lab for debugging; the leftover lab is then reported by whoever runs the suite.
async fn cleanup_lab(
    ws: &mut TestWsClient,
    server: &TestServer,
    token: &str,
    lab_id: &str,
    checks: &Result<()>,
) -> Result<()> {
    if checks.is_err() && std::env::var("SHERPA_TEST_KEEP_FAILED_LABS").as_deref() == Ok("1") {
        eprintln!("Keeping failed lab {lab_id} for debugging");
        return Ok(());
    }
    ensure_lab_destroyed(ws, server, token, lab_id).await
}

/// Report a test failure and a cleanup failure together; neither may hide the other.
fn with_cleanup(checks: Result<()>, cleanup: Result<()>) -> Result<()> {
    match (checks, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(cleanup)) => Err(cleanup.context("Test passed but lab cleanup failed")),
        (Err(error), Err(cleanup)) => {
            Err(error.context(format!("Lab cleanup also failed: {cleanup:#}")))
        }
    }
}

/// Log in to `address` as `user` with `password` and run `true`. A connection or login that
/// fails counts as `false`, as the guest may still be provisioning.
async fn password_login(address: &str, user: &str, password: &str) -> Result<bool> {
    let hop = Hop::new(
        Node::new(address, SSH_PORT),
        Auth::new(user).password(Secret::new(password.to_owned())),
        HostKeyPolicy::Insecure,
    );
    let mut options = ConnectOptions::default();
    options.connect_timeout = Duration::from_secs(10);
    let session = match Session::connect(&hop, &options).await {
        Ok(session) => session,
        Err(error) => {
            eprintln!("SSH login to {user}@{address} failed: {error}");
            return Ok(false);
        }
    };
    let output = session
        .run("true", b"", Duration::from_secs(30))
        .await
        .context("Failed to run a command over SSH")?;
    if let Err(error) = session.close().await {
        eprintln!("SSH session to {user}@{address} did not close cleanly: {error}");
    }
    Ok(output.exit == Exit::Code(0))
}

/// Retry a login that should succeed until the guest has finished provisioning it.
async fn wait_for_password_login(
    address: &str,
    user: &str,
    password: &str,
    timeout: Duration,
) -> Result<bool> {
    let start = Instant::now();
    loop {
        if password_login(address, user, password).await? {
            return Ok(true);
        }
        if start.elapsed() >= timeout {
            return Ok(false);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// The management address of node `name` of lab `lab_id`, as recorded.
async fn node_address(server: &TestServer, lab_id: &str, name: &str) -> Result<String> {
    let lab = db::get_lab(&server.db, lab_id).await?;
    let node = db::get_node_by_name_and_lab(&server.db, name, db::get_lab_id(&lab)?).await?;
    node.mgmt_ipv4
        .with_context(|| format!("node {name} has no management address"))
}

/// What lab `lab_id` left in libvirt: its domains, networks and storage volumes, by name.
fn libvirt_leftovers(lab_id: &str) -> Result<Vec<String>> {
    let conn = libvirt::Qemu::default().connect()?;
    let mut leftovers: Vec<String> = libvirt::list_domain_memory(&conn)?
        .into_iter()
        .map(|domain| domain.name)
        .filter(|name| name.contains(lab_id))
        .collect();
    for network in conn
        .list_all_networks(0)
        .context("Failed to list libvirt networks")?
    {
        let name = network
            .get_name()
            .context("Failed to read a libvirt network's name")?;
        if name.contains(lab_id) {
            leftovers.push(name);
        }
    }
    leftovers.extend(libvirt::list_lab_volumes(&conn, lab_id)?);
    Ok(leftovers)
}

/// Each node's recorded state, by name.
async fn node_states(server: &TestServer, lab_id: &str) -> Result<BTreeMap<String, NodeState>> {
    let lab = db::get_lab(&server.db, lab_id).await?;
    let nodes = db::list_nodes_by_lab(&server.db, lab.id.context("lab has no record ID")?).await?;
    Ok(nodes
        .into_iter()
        .map(|node| (node.name, node.state))
        .collect())
}

/// The status of lab `lab_id`, derived from its nodes as the server derives it when read.
async fn lab_status(server: &TestServer, lab_id: &str) -> Result<LabState> {
    let states: Vec<NodeState> = node_states(server, lab_id).await?.into_values().collect();
    Ok(LabState::derive(&states))
}

/// Wait until node `name` of lab `lab_id` is recorded in `state`, as the scanner records it.
async fn wait_for_node_state(
    server: &TestServer,
    lab_id: &str,
    name: &str,
    state: NodeState,
    timeout: Duration,
) -> Result<()> {
    let start = Instant::now();
    loop {
        let states = node_states(server, lab_id).await?;
        if states.get(name) == Some(&state) {
            return Ok(());
        }
        if start.elapsed() > timeout {
            bail!("node {name} was not recorded {state} within {timeout:?}: {states:?}");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn lab_up(
    ws: &mut TestWsClient,
    token: &str,
    lab_id: &str,
    manifest: &serde_json::Value,
    timeout: Duration,
) -> Result<Vec<serde_json::Value>> {
    let (statuses, response) = ws
        .rpc_call_streaming(
            "up",
            json!({ "token": token, "lab_id": lab_id, "manifest": manifest }),
            timeout,
        )
        .await?;
    ensure!(
        response.get("error").is_none(),
        "lab up should succeed: {response}"
    );
    Ok(statuses)
}

// ── Container Lab Lifecycle ──

#[tokio::test]
#[ignore = "requires Docker and host network administration rights (CAP_NET_ADMIN)"]
async fn lab_up_004_a_node_that_fails_to_start_fails_the_node_not_the_lab_and_up_exits_non_zero()
-> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    bootstrap_images(&mut ws, &token).await?;

    // A generic container keeps the manifest's commands; alpine is in the dev server's Docker.
    let mut alpine = ModelKind::new(NodeModel::Generic, Some(NodeKind::Container))
        .unwrap()
        .node_config();
    alpine.settings = KindSettings::Container(ContainerSettings {
        repo: Some("alpine".to_owned()),
        ztp: ZtpSettings::default(),
    });
    alpine.version = "latest".to_owned();
    alpine.default = true;
    db::upsert_node_image(&server.db, alpine, DefaultRequest::Keep).await?;

    let lab_name = format!("tx{:06}", std::process::id() % 1_000_000);
    let manifest = json!({
        "name": lab_name,
        "nodes": [
            { "name": "dead1", "model": "generic", "kind": "container", "container": { "commands": ["/bin/false"] }, "interfaces": { "data_count": 0 } },
            { "name": "live1", "model": "generic", "kind": "container", "container": { "commands": ["sleep", "600"] }, "interfaces": { "data_count": 0 } },
        ]
    });

    let checks: Result<()> = async {
        let (_statuses, response) = ws
            .rpc_call_streaming(
                "up",
                json!({ "token": token, "lab_id": lab_name, "manifest": manifest }),
                Duration::from_secs(180),
            )
            .await?;
        ensure!(
            response.get("error").is_none(),
            "the lab was rolled back: {response}"
        );
        let errors = response["result"]["errors"].to_string();
        ensure!(
            errors.contains("dead1") && errors.contains("exited"),
            "the exited node was not reported: {response}"
        );
        // M6.1: a lab with a node that will not recover is only partly created.
        ensure!(
            response["result"]["success"] == json!(false),
            "up reported success with a failed node: {response}"
        );
        ensure!(
            db::get_lab(&server.db, &lab_name).await?.phase == LabPhase::Ready,
            "the partly created lab is not ready to use or destroy"
        );
        ensure!(
            db::find_lab(&server.db, &lab_name).await?.is_some(),
            "the lab record is gone"
        );
        // The row and the response agree: the exited node is failed, the other running.
        let states = node_states(&server, &lab_name).await?;
        ensure!(
            states.get("dead1") == Some(&NodeState::Failed)
                && states.get("live1") == Some(&NodeState::Running),
            "unexpected node states: {states:?}"
        );
        ensure!(
            lab_status(&server, &lab_name).await? == LabState::Failed,
            "the lab status does not follow its nodes"
        );
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab_name, &checks).await;
    with_cleanup(checks, cleanup)
}

/// Register alpine as the default `generic` container image, which keeps the manifest's commands.
async fn register_alpine(server: &TestServer) -> Result<()> {
    let alpine = NodeConfig {
        settings: KindSettings::Container(ContainerSettings {
            repo: Some("alpine".to_owned()),
            ztp: ZtpSettings::default(),
        }),
        version: "latest".to_owned(),
        default: true,
        ..ModelKind::new(NodeModel::Generic, Some(NodeKind::Container))
            .unwrap()
            .node_config()
    };
    db::upsert_node_image(&server.db, alpine, DefaultRequest::Keep).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker and host network administration rights (CAP_NET_ADMIN)"]
async fn lab_up_005_a_container_that_exits_shortly_after_starting_fails_with_its_exit_code_and_log()
-> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    bootstrap_images(&mut ws, &token).await?;
    register_alpine(&server).await?;

    // Like mysql without a root password: running when Docker first reports it, gone a moment
    // later, with a reason in its log.
    let lab_name = format!("te{:06}", std::process::id() % 1_000_000);
    let manifest = json!({
        "name": lab_name,
        "nodes": [
            { "name": "late1", "model": "generic", "kind": "container", "container": { "commands": ["sh", "-c", "echo password option is not specified; sleep 1; exit 3"] }, "interfaces": { "data_count": 0 } },
            { "name": "live1", "model": "generic", "kind": "container", "container": { "commands": ["sleep", "600"] }, "interfaces": { "data_count": 0 } },
        ]
    });

    let checks: Result<()> = async {
        let (_statuses, response) = ws
            .rpc_call_streaming(
                "up",
                json!({ "token": token, "lab_id": lab_name, "manifest": manifest }),
                Duration::from_secs(180),
            )
            .await?;
        ensure!(
            response.get("error").is_none(),
            "the lab was rolled back: {response}"
        );
        let errors = response["result"]["errors"].to_string();
        ensure!(
            errors.contains("Node late1 exited with code 3 before it was ready"),
            "the exit code was not reported: {response}"
        );
        ensure!(
            errors.contains("password option is not specified"),
            "the container's log was not reported: {response}"
        );
        ensure!(
            response["result"]["success"] == json!(false),
            "up reported success with a failed node: {response}"
        );
        let states = node_states(&server, &lab_name).await?;
        ensure!(
            states.get("late1") == Some(&NodeState::Failed)
                && states.get("live1") == Some(&NodeState::Running),
            "unexpected node states: {states:?}"
        );
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab_name, &checks).await;
    with_cleanup(checks, cleanup)
}

#[tokio::test]
#[ignore = "requires Docker and host network administration rights (CAP_NET_ADMIN)"]
async fn inspect_reports_a_container_that_exited_after_up_as_docker_has_it() -> Result<()> {
    // No scanner, so the record keeps what `up` wrote.
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    bootstrap_images(&mut ws, &token).await?;
    register_alpine(&server).await?;

    let lab_name = format!("ti{:06}", std::process::id() % 1_000_000);
    let manifest = json!({
        "name": lab_name,
        "nodes": [
            { "name": "live1", "model": "generic", "kind": "container", "container": { "commands": ["sleep", "600"] }, "interfaces": { "data_count": 0 } },
        ]
    });
    let docker = Docker::connect_with_local_defaults()?;

    let checks: Result<()> = async {
        lab_up(
            &mut ws,
            &token,
            &lab_name,
            &manifest,
            Duration::from_secs(180),
        )
        .await?;
        container::kill_container(&docker, &format!("live1-{lab_name}")).await?;

        let response = ws
            .rpc_call("inspect", json!({ "token": token, "lab_id": lab_name }))
            .await?;
        let device = &response["result"]["devices"][0];
        ensure!(
            device["name"] == "live1" && device["state"] == "stopped",
            "inspect should report the exited container stopped: {response}"
        );
        ensure!(
            response["result"]["lab_state"] == "stopped",
            "the lab status should follow the reported states: {response}"
        );
        // Inspect reports; the scanner records.
        let states = node_states(&server, &lab_name).await?;
        ensure!(
            states.get("live1") == Some(&NodeState::Running),
            "inspect should not write the node's state: {states:?}"
        );
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab_name, &checks).await;
    with_cleanup(checks, cleanup)
}

#[tokio::test]
#[ignore = "requires Docker and host network administration rights (CAP_NET_ADMIN)"]
async fn lab_clean_001_clean_removes_a_running_lab() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    let admin_token = ws.login_admin().await?;
    bootstrap_images(&mut ws, &token).await?;

    // A generic container keeps the manifest's commands; alpine is in the dev server's Docker.
    let mut alpine = ModelKind::new(NodeModel::Generic, Some(NodeKind::Container))
        .unwrap()
        .node_config();
    alpine.settings = KindSettings::Container(ContainerSettings {
        repo: Some("alpine".to_owned()),
        ztp: ZtpSettings::default(),
    });
    alpine.version = "latest".to_owned();
    alpine.default = true;
    db::upsert_node_image(&server.db, alpine, DefaultRequest::Keep).await?;

    let lab_name = format!("tk{:06}", std::process::id() % 1_000_000);
    let manifest = json!({
        "name": lab_name,
        "nodes": [
            { "name": "box1", "model": "generic", "kind": "container", "container": { "commands": ["sleep", "600"] }, "interfaces": { "data_count": 0 } },
        ]
    });

    let checks: Result<()> = async {
        lab_up(
            &mut ws,
            &token,
            &lab_name,
            &manifest,
            Duration::from_secs(180),
        )
        .await?;
        let docker = Docker::connect_with_local_defaults()?;
        let container = format!("box1-{lab_name}");
        ensure!(
            docker
                .inspect_container(&container, None::<InspectContainerOptions>)
                .await
                .is_ok(),
            "the lab's container was not created"
        );

        let response = ws
            .rpc_call("clean", json!({ "token": admin_token, "lab_id": lab_name }))
            .await?;
        ensure!(response.get("error").is_none(), "clean failed: {response}");
        ensure!(
            docker
                .inspect_container(&container, None::<InspectContainerOptions>)
                .await
                .is_err(),
            "clean left the lab's container"
        );
        let networks = docker.list_networks(None::<ListNetworksOptions>).await?;
        ensure!(
            !networks
                .iter()
                .filter_map(|network| network.name.as_deref())
                .any(|name| name.ends_with(&format!("-{lab_name}"))),
            "clean left a Docker network of the lab"
        );
        ensure!(
            db::find_lab(&server.db, &lab_name).await?.is_none(),
            "clean left the lab record"
        );
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab_name, &checks).await;
    with_cleanup(checks, cleanup)
}

#[tokio::test]
#[ignore = "requires Docker, internet access to pull an image and host network administration rights (CAP_NET_ADMIN)"]
async fn an_official_docker_hub_image_pulled_by_its_full_name_brings_a_container_node_up()
-> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    let admin_token = ws.login_admin().await?;

    // Docker tags this image `busybox:1.36`, not by the full name it was pulled as.
    let repo = "docker.io/library/busybox";
    let (_statuses, response) = ws
        .rpc_call_streaming(
            "image.pull",
            json!({
                "token": admin_token,
                "model": "generic", "kind": "container",
                "repo": repo,
                "tag": "1.36",
                "default": true,
            }),
            Duration::from_secs(300),
        )
        .await?;
    ensure!(
        response.get("error").is_none(),
        "image.pull failed: {response}"
    );

    let lab_name = format!("tb{:06}", std::process::id() % 1_000_000);
    let manifest = json!({
        "name": lab_name,
        "nodes": [
            { "name": "box1", "model": "generic", "kind": "container", "container": { "commands": ["sleep", "600"] }, "interfaces": { "data_count": 0 } },
        ]
    });

    let checks: Result<()> = async {
        lab_up(
            &mut ws,
            &token,
            &lab_name,
            &manifest,
            Duration::from_secs(180),
        )
        .await?;
        let docker = Docker::connect_with_local_defaults()?;
        let container = format!("box1-{lab_name}");
        let (_, image) = container_image(&docker, &container).await?;
        ensure!(
            image.as_deref() == Some("docker.io/library/busybox:1.36"),
            "the container did not start from the image that was pulled and checked: {image:?}"
        );

        // A redeploy resolves, checks and creates the same reference.
        let (_statuses, response) = ws
            .rpc_call_streaming(
                "redeploy",
                json!({ "token": token, "lab_id": lab_name, "node_name": "box1", "manifest": manifest }),
                Duration::from_secs(180),
            )
            .await?;
        ensure!(
            response.get("error").is_none(),
            "the redeploy failed: {response}"
        );
        let (redeployed_id, image) = container_image(&docker, &container).await?;
        ensure!(
            image.as_deref() == Some("docker.io/library/busybox:1.36"),
            "the redeployed container did not start from the checked image: {image:?}"
        );

        // An image Docker does not have refuses the redeploy before the node is torn down.
        let mut missing = manifest.clone();
        missing["nodes"][0]["image"] = json!("sherpa-test/no-such-image");
        let (_statuses, response) = ws
            .rpc_call_streaming(
                "redeploy",
                json!({ "token": token, "lab_id": lab_name, "node_name": "box1", "manifest": missing }),
                Duration::from_secs(180),
            )
            .await?;
        ensure!(
            response.to_string().contains("sherpa-test/no-such-image:1.36"),
            "the redeploy was not refused for the missing image: {response}"
        );
        let (kept_id, _) = container_image(&docker, &container).await?;
        ensure!(
            kept_id == redeployed_id,
            "the refused redeploy replaced the container"
        );
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab_name, &checks).await;
    with_cleanup(checks, cleanup)
}

#[tokio::test]
#[ignore = "requires Docker and host network administration rights (CAP_NET_ADMIN)"]
async fn lab_redeploy_005_a_redeploy_that_fails_after_teardown_fails_its_node() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    bootstrap_images(&mut ws, &token).await?;

    // A generic container keeps the manifest's commands; alpine is in the dev server's Docker.
    let mut alpine = ModelKind::new(NodeModel::Generic, Some(NodeKind::Container))
        .unwrap()
        .node_config();
    alpine.settings = KindSettings::Container(ContainerSettings {
        repo: Some("alpine".to_owned()),
        ztp: ZtpSettings::default(),
    });
    alpine.version = "latest".to_owned();
    alpine.default = true;
    db::upsert_node_image(&server.db, alpine, DefaultRequest::Keep).await?;

    let lab_name = format!("tf{:06}", std::process::id() % 1_000_000);
    let manifest = json!({
        "name": lab_name,
        "nodes": [
            { "name": "box1", "model": "generic", "kind": "container", "container": { "commands": ["sleep", "600"] }, "interfaces": { "data_count": 0 } },
        ]
    });
    // Docker creates the container, then cannot start it: the node is already torn down.
    let broken = json!({
        "name": lab_name,
        "nodes": [
            { "name": "box1", "model": "generic", "kind": "container", "container": { "commands": ["sleep", "600"], "user": "nosuchuser" }, "interfaces": { "data_count": 0 } },
        ]
    });

    let checks: Result<()> = async {
        lab_up(&mut ws, &token, &lab_name, &manifest, Duration::from_secs(180)).await?;
        let (_statuses, response) = ws
            .rpc_call_streaming(
                "redeploy",
                json!({ "token": token, "lab_id": lab_name, "node_name": "box1", "manifest": broken }),
                Duration::from_secs(180),
            )
            .await?;
        ensure!(
            response.get("error").is_some(),
            "the redeploy succeeded: {response}"
        );
        let lab = db::get_lab(&server.db, &lab_name).await?;
        let node = db::get_node_by_name_and_lab(&server.db, "box1", db::get_lab_id(&lab)?).await?;
        ensure!(
            node.state == NodeState::Failed,
            "the node was left {}",
            node.state
        );
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab_name, &checks).await;
    with_cleanup(checks, cleanup)
}

/// The FRR image the ZTP tests run, which must be in the server's Docker.
const FRR_TEST_VERSION: &str = "10.5.2";

/// Where Sherpa mounts the config files it generates for an FRR node.
const FRR_ZTP_MOUNTS: [&str; 3] = [
    "/etc/frr/daemons",
    "/etc/frr/frr.conf",
    "/tmp/sherpa-init.sh",
];

/// The paths mounted into a lab node's container, and the command it was started with.
async fn container_mounts_and_command(
    docker: &Docker,
    container: &str,
) -> Result<(Vec<String>, Vec<String>)> {
    let inspect = docker
        .inspect_container(container, None::<InspectContainerOptions>)
        .await
        .with_context(|| format!("Failed to inspect container {container}"))?;
    let mounts = inspect
        .mounts
        .unwrap_or_default()
        .into_iter()
        .filter_map(|mount| mount.destination)
        .collect();
    let command = inspect
        .config
        .and_then(|config| config.cmd)
        .unwrap_or_default();
    Ok((mounts, command))
}

/// An FRR node with ZTP on has Sherpa's three config mounts and runs its startup script; one
/// with ZTP off has none of them and runs the image's own command.
async fn check_frr_ztp(docker: &Docker, lab_name: &str) -> Result<()> {
    let (mounts, command) = container_mounts_and_command(docker, &format!("on-{lab_name}")).await?;
    for mount in FRR_ZTP_MOUNTS {
        ensure!(
            mounts.iter().any(|path| path == mount),
            "the node with ZTP on has no {mount} mount: {mounts:?}"
        );
    }
    ensure!(
        command == ["/bin/sh", "/tmp/sherpa-init.sh"],
        "the node with ZTP on runs {command:?}"
    );

    let (mounts, command) =
        container_mounts_and_command(docker, &format!("off-{lab_name}")).await?;
    for mount in FRR_ZTP_MOUNTS {
        ensure!(
            !mounts.iter().any(|path| path == mount),
            "the node with ZTP off has a {mount} mount: {mounts:?}"
        );
    }
    ensure!(
        mounts.iter().any(|path| path == "/etc/own.conf"),
        "the node with ZTP off lost the manifest's volume: {mounts:?}"
    );
    ensure!(
        !command.iter().any(|part| part.contains("sherpa-init")),
        "the node with ZTP off runs {command:?}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker with quay.io/frrouting/frr:10.5.2 and host network administration rights (CAP_NET_ADMIN)"]
async fn frr_gets_its_config_mounts_and_command_only_with_ztp_on() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    bootstrap_images(&mut ws, &token).await?;

    let mut frr = ModelKind::default_for(NodeModel::FrrLinux).node_config();
    frr.version = FRR_TEST_VERSION.to_owned();
    db::upsert_node_image(&server.db, frr, DefaultRequest::Make).await?;

    let lab_name = format!("tz{:06}", std::process::id() % 1_000_000);
    let own_config = std::env::temp_dir().join(format!("{lab_name}-own.conf"));
    fs::write(&own_config, "! the user's own config\n")?;
    let manifest = json!({
        "name": lab_name,
        "nodes": [
            { "name": "on", "model": "frr_linux" },
            {
                "name": "off",
                "model": "frr_linux",
                "ztp": { "method": "disabled" },
                "container": { "volumes": [{ "src": own_config, "dst": "/etc/own.conf" }] },
            },
        ]
    });

    let checks: Result<()> = async {
        lab_up(&mut ws, &token, &lab_name, &manifest, Duration::from_secs(180)).await?;
        let docker = Docker::connect_with_local_defaults()?;
        check_frr_ztp(&docker, &lab_name).await?;

        for node in ["on", "off"] {
            let (_statuses, response) = ws
                .rpc_call_streaming(
                    "redeploy",
                    json!({ "token": token, "lab_id": lab_name, "node_name": node, "manifest": manifest }),
                    Duration::from_secs(180),
                )
                .await?;
            ensure!(
                response.get("error").is_none(),
                "redeploy of {node} failed: {response}"
            );
        }
        check_frr_ztp(&docker, &lab_name).await?;

        // A method the model does not take is refused before the node is torn down.
        let mut unsupported = manifest.clone();
        unsupported["nodes"][0]["ztp"] = json!({ "method": "cdrom" });
        let (_statuses, response) = ws
            .rpc_call_streaming(
                "redeploy",
                json!({ "token": token, "lab_id": lab_name, "node_name": "on", "manifest": unsupported }),
                Duration::from_secs(60),
            )
            .await?;
        ensure!(
            response.to_string().contains("allows: disabled, volume"),
            "the redeploy was not refused for its ZTP setting: {response}"
        );
        check_frr_ztp(&docker, &lab_name).await
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab_name, &checks).await;
    fs::remove_file(&own_config).ok();
    with_cleanup(checks, cleanup)
}

#[tokio::test]
#[ignore = "requires Docker with quay.io/frrouting/frr:10.5.2 and host network administration rights (CAP_NET_ADMIN)"]
async fn lab_preflight_004_an_unsupported_ztp_setting_is_refused() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    bootstrap_images(&mut ws, &token).await?;

    let mut frr = ModelKind::default_for(NodeModel::FrrLinux).node_config();
    frr.version = FRR_TEST_VERSION.to_owned();
    db::upsert_node_image(&server.db, frr, DefaultRequest::Make).await?;

    let lab_name = format!("ty{:06}", std::process::id() % 1_000_000);
    let unsupported = json!({
        "name": lab_name,
        "nodes": [{ "name": "frr1", "model": "frr_linux", "ztp": { "method": "cdrom" } }]
    });
    // A config the node's setting would not use is refused the same way.
    let unused_config = json!({
        "name": lab_name,
        "nodes": [{ "name": "frr1", "model": "frr_linux", "ztp": { "method": "disabled", "config": "IQ==" } }]
    });

    let checks: Result<()> = async {
        let docker = Docker::connect_with_local_defaults()?;
        for (manifest, reason) in [
            (&unsupported, "allows: disabled, volume"),
            (&unused_config, "its ZTP is disabled"),
        ] {
            let (_statuses, response) = ws
                .rpc_call_streaming(
                    "up",
                    json!({ "token": token, "lab_id": lab_name, "manifest": manifest }),
                    Duration::from_secs(60),
                )
                .await?;
            ensure!(
                response.to_string().contains(reason),
                "up was not refused with '{reason}': {response}"
            );
            ensure!(
                db::get_lab(&server.db, &lab_name).await.is_err(),
                "the refused up left a lab record"
            );
            ensure!(
                docker
                    .inspect_container(&format!("frr1-{lab_name}"), None::<InspectContainerOptions>)
                    .await
                    .is_err(),
                "the refused up created the node's container"
            );
            let networks = docker.list_networks(None::<ListNetworksOptions>).await?;
            ensure!(
                !networks
                    .iter()
                    .filter_map(|network| network.name.as_deref())
                    .any(|name| name.ends_with(&format!("-{lab_name}"))),
                "the refused up left a Docker network"
            );
        }
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab_name, &checks).await;
    with_cleanup(checks, cleanup)
}

#[tokio::test]
#[ignore = "requires Docker and host network administration rights (CAP_NET_ADMIN)"]
async fn the_scanner_records_what_docker_reports_but_not_over_up() -> Result<()> {
    let server = TestServer::start_with_scanner(1).await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    bootstrap_images(&mut ws, &token).await?;

    // A generic container keeps the manifest's commands; alpine is in the dev server's Docker.
    let mut alpine = ModelKind::new(NodeModel::Generic, Some(NodeKind::Container))
        .unwrap()
        .node_config();
    alpine.settings = KindSettings::Container(ContainerSettings {
        repo: Some("alpine".to_owned()),
        ztp: ZtpSettings::default(),
    });
    alpine.version = "latest".to_owned();
    alpine.default = true;
    db::upsert_node_image(&server.db, alpine, DefaultRequest::Keep).await?;

    let lab_name = format!("sc{:06}", std::process::id() % 1_000_000);
    let manifest = json!({
        "name": lab_name,
        "nodes": [
            { "name": "live1", "model": "generic", "kind": "container", "container": { "commands": ["sleep", "600"] }, "interfaces": { "data_count": 0 } },
            { "name": "live2", "model": "generic", "kind": "container", "container": { "commands": ["sleep", "600"] }, "interfaces": { "data_count": 0 } },
        ]
    });
    let docker = Docker::connect_with_local_defaults()?;
    let container = format!("live1-{lab_name}");

    let checks: Result<()> = async {
        // M1.1: while `up` runs, the scanner must not record the lab or its nodes stopped.
        let up_done = Arc::new(AtomicBool::new(false));
        let watcher = {
            let db = server.db.clone();
            let lab_name = lab_name.clone();
            let up_done = up_done.clone();
            tokio::spawn(async move {
                let mut seen = Vec::new();
                while !up_done.load(Ordering::SeqCst) {
                    if let Ok(lab) = db::get_lab(&db, &lab_name).await
                        && let Some(id) = lab.id
                        && let Ok(nodes) = db::list_nodes_by_lab(&db, id).await
                    {
                        let states: Vec<NodeState> = nodes.iter().map(|node| node.state).collect();
                        seen.push(LabState::derive(&states));
                        seen.extend(nodes.iter().filter_map(|node| {
                            matches!(node.state, NodeState::Stopped | NodeState::Missing)
                                .then_some(LabState::Stopped)
                        }));
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                seen
            })
        };
        let up = lab_up(
            &mut ws,
            &token,
            &lab_name,
            &manifest,
            Duration::from_secs(180),
        )
        .await;
        up_done.store(true, Ordering::SeqCst);
        up?;
        let seen = watcher.await?;
        ensure!(
            !seen.contains(&LabState::Stopped),
            "the lab or a node was recorded stopped while up ran: {seen:?}"
        );

        let wait = Duration::from_secs(20);
        wait_for_node_state(&server, &lab_name, "live1", NodeState::Running, wait).await?;
        // Changes made outside Sherpa are recorded by the scanner.
        container::stop_container(&docker, &container).await?;
        wait_for_node_state(&server, &lab_name, "live1", NodeState::Stopped, wait).await?;
        container::start_container(&docker, &container).await?;
        wait_for_node_state(&server, &lab_name, "live1", NodeState::Running, wait).await?;
        // M4.2: a container that is gone is missing, not stopped, and the lab shows it.
        container::kill_container(&docker, &container).await?;
        container::remove_container(&docker, &container).await?;
        wait_for_node_state(&server, &lab_name, "live1", NodeState::Missing, wait).await?;
        ensure!(
            lab_status(&server, &lab_name).await? == LabState::Failed,
            "a lab with a missing node should be failed"
        );
        // Resume refuses it with a way out.
        let (_s, response) = ws
            .rpc_call_streaming(
                "resume",
                json!({ "token": token, "lab_id": lab_name, "node_name": "live1" }),
                Duration::from_secs(60),
            )
            .await?;
        let result: LabNodeActionResponse = serde_json::from_value(response["result"].clone())
            .with_context(|| format!("resume response: {response}"))?;
        ensure!(
            result
                .results
                .iter()
                .any(|node| !node.success && node.message.contains("redeploy")),
            "resume of a missing node should say to redeploy it: {response}"
        );
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab_name, &checks).await;
    with_cleanup(checks, cleanup)
}

#[tokio::test]
#[ignore = "requires Docker and host network administration rights (CAP_NET_ADMIN)"]
async fn lab_up_001_a_container_lab_comes_up_with_every_node_running() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    bootstrap_images(&mut ws, &token).await?;

    // lab_id must be exactly 8 characters
    let lab_name = format!("tc{:06}", std::process::id() % 1_000_000);
    let description = "Practice BGP.\nTwo routers, café";
    let mut manifest = container_manifest(&lab_name);
    manifest["description"] = json!(description);

    let checks: Result<()> = async {
        let statuses = lab_up(
            &mut ws,
            &token,
            &lab_name,
            &manifest,
            Duration::from_secs(180),
        )
        .await?;
        ensure!(
            !statuses.is_empty(),
            "should have status messages during up"
        );
        let inspect_resp = ws
            .rpc_call("inspect", json!({ "token": token, "lab_id": lab_name }))
            .await?;
        ensure!(
            inspect_resp.get("result").is_some(),
            "inspect should succeed: {inspect_resp}"
        );
        // The description goes from the manifest into the lab's record and back out.
        ensure!(
            inspect_resp["result"]["lab_info"]["description"] == description,
            "inspect should return the description: {inspect_resp}"
        );
        let labs = ws.rpc_call("labs.list", json!({ "token": token })).await?;
        ensure!(
            labs["result"]["labs"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|lab| lab["id"] == lab_name.as_str() && lab["description"] == description),
            "labs.list should return the description: {labs}"
        );
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab_name, &checks).await;
    with_cleanup(checks, cleanup)
}

#[tokio::test]
#[ignore = "requires Docker and host network administration rights (CAP_NET_ADMIN)"]
async fn lab_down_resume_001_a_container_lab_goes_down_and_resumes_with_its_nodes_running()
-> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    bootstrap_images(&mut ws, &token).await?;

    // lab_id must be exactly 8 characters
    let lab_name = format!("dr{:06}", std::process::id() % 1_000_000);
    let manifest = container_manifest(&lab_name);

    let checks: Result<()> = async {
        lab_up(
            &mut ws,
            &token,
            &lab_name,
            &manifest,
            Duration::from_secs(180),
        )
        .await?;
        // Each runs twice: a second down or resume finds the nodes already there.
        for method in ["down", "down", "resume", "resume"] {
            let (_s, response) = ws
                .rpc_call_streaming(
                    method,
                    json!({ "token": token, "lab_id": lab_name }),
                    Duration::from_secs(300),
                )
                .await?;
            ensure!(
                response.get("error").is_none(),
                "{method} failed: {response}"
            );
            let result: LabNodeActionResponse = serde_json::from_value(response["result"].clone())?;
            ensure!(
                result.results.iter().all(|node| node.success),
                "{method} should succeed for every node: {response}"
            );
            // No scanner runs here: the operation records the states itself.
            let (node, lab) = if method == "down" {
                (NodeState::Stopped, LabState::Stopped)
            } else {
                (NodeState::Running, LabState::Running)
            };
            let states = node_states(&server, &lab_name).await?;
            ensure!(
                states.values().all(|state| *state == node),
                "after {method} every node should be {node}: {states:?}"
            );
            ensure!(
                lab_status(&server, &lab_name).await? == lab,
                "after {method} the lab should be {lab}"
            );
        }
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab_name, &checks).await;
    with_cleanup(checks, cleanup)
}

#[tokio::test]
#[ignore = "requires Docker and host network administration rights (CAP_NET_ADMIN)"]
async fn lab_destroy_001_a_container_lab_is_destroyed_with_nothing_left_in_docker_the_database_or_the_lab_folder()
-> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    bootstrap_images(&mut ws, &token).await?;

    // lab_id must be exactly 8 characters
    let lab_name = format!("ds{:06}", std::process::id() % 1_000_000);
    let manifest = container_manifest(&lab_name);

    let checks: Result<()> = async {
        lab_up(
            &mut ws,
            &token,
            &lab_name,
            &manifest,
            Duration::from_secs(180),
        )
        .await?;
        let (_s, destroy_resp) = ws
            .rpc_call_streaming(
                "destroy",
                json!({ "token": token, "lab_id": lab_name }),
                Duration::from_secs(60),
            )
            .await?;
        ensure!(
            destroy_resp.get("error").is_none(),
            "destroy should succeed: {destroy_resp}"
        );
        let inspect_resp = ws
            .rpc_call("inspect", json!({ "token": token, "lab_id": lab_name }))
            .await?;
        ensure!(
            inspect_resp.get("error").is_some(),
            "inspect after destroy should fail"
        );
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab_name, &checks).await;
    with_cleanup(checks, cleanup)
}

/// Add the alpine `generic` container image, which keeps the manifest's commands, and return a
/// one-node manifest for lab `lab_name` that sleeps.
async fn sleeping_container_lab(server: &TestServer, lab_name: &str) -> Result<serde_json::Value> {
    let mut alpine = ModelKind::new(NodeModel::Generic, Some(NodeKind::Container))
        .unwrap()
        .node_config();
    alpine.settings = KindSettings::Container(ContainerSettings {
        repo: Some("alpine".to_owned()),
        ztp: ZtpSettings::default(),
    });
    "latest".clone_into(&mut alpine.version);
    alpine.default = true;
    db::upsert_node_image(&server.db, alpine, DefaultRequest::Keep).await?;
    Ok(json!({
        "name": lab_name,
        "nodes": [
            { "name": "box1", "model": "generic", "kind": "container", "container": { "commands": ["sleep", "600"] }, "interfaces": { "data_count": 0 } },
        ]
    }))
}

/// M3.1: a second `up` of the same lab used to fail in phase 3 and roll back the first one's
/// lab. Now it is refused at the reservation and touches nothing.
#[tokio::test]
#[ignore = "requires Docker and host network administration rights (CAP_NET_ADMIN)"]
async fn lab_concurrency_001_a_second_up_of_a_lab_that_is_coming_up_is_refused_and_leaves_the_first_alone()
-> Result<()> {
    let server = TestServer::start().await?;
    let mut first = TestWsClient::connect(&server).await?;
    let mut second = TestWsClient::connect(&server).await?;
    let token = first.login_lab_admin(&server).await?;
    bootstrap_images(&mut first, &token).await?;
    let lab_name = format!("tu{:06}", std::process::id() % 1_000_000);
    let manifest = sleeping_container_lab(&server, &lab_name).await?;
    let request = json!({ "token": token, "lab_id": lab_name, "manifest": manifest });

    let (first_up, second_up) = tokio::join!(
        first.rpc_call_streaming("up", request.clone(), Duration::from_secs(180)),
        second.rpc_call_streaming("up", request.clone(), Duration::from_secs(180)),
    );
    let checks: Result<()> = async {
        let responses = [first_up?.1, second_up?.1];
        let refused: Vec<&serde_json::Value> = responses
            .iter()
            .filter(|response| response.get("error").is_some())
            .collect();
        ensure!(
            refused.len() == 1,
            "exactly one up should be refused: {responses:?}"
        );
        ensure!(
            refused[0].to_string().contains("already exists"),
            "the refusal does not say why: {}",
            refused[0]
        );
        let lab = db::get_lab(&server.db, &lab_name).await?;
        ensure!(lab.phase == LabPhase::Ready, "the lab is {:?}", lab.phase);
        let docker = Docker::connect_with_local_defaults()?;
        let container = docker
            .inspect_container(&format!("box1-{lab_name}"), None::<InspectContainerOptions>)
            .await?;
        ensure!(
            container.state.and_then(|state| state.running) == Some(true),
            "the refused up removed the first up's container"
        );
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut first, &server, &token, &lab_name, &checks).await;
    with_cleanup(checks, cleanup)
}

/// M7.2: the second of two destroys used to fail on the record the first had removed.
#[tokio::test]
#[ignore = "requires Docker and host network administration rights (CAP_NET_ADMIN)"]
async fn lab_concurrency_002_two_destroys_of_one_lab_both_succeed() -> Result<()> {
    let server = TestServer::start().await?;
    let mut first = TestWsClient::connect(&server).await?;
    let mut second = TestWsClient::connect(&server).await?;
    let token = first.login_lab_admin(&server).await?;
    bootstrap_images(&mut first, &token).await?;
    let lab_name = format!("td{:06}", std::process::id() % 1_000_000);
    let manifest = sleeping_container_lab(&server, &lab_name).await?;
    lab_up(
        &mut first,
        &token,
        &lab_name,
        &manifest,
        Duration::from_secs(180),
    )
    .await?;

    let request = json!({ "token": token, "lab_id": lab_name });
    let (first_destroy, second_destroy) = tokio::join!(
        first.rpc_call_streaming("destroy", request.clone(), Duration::from_secs(300)),
        second.rpc_call_streaming("destroy", request.clone(), Duration::from_secs(300)),
    );
    let checks: Result<()> = async {
        for (_, response) in [first_destroy?, second_destroy?] {
            ensure!(
                response.get("error").is_none() && response["result"]["success"] == json!(true),
                "a destroy failed: {response}"
            );
        }
        ensure!(
            db::find_lab(&server.db, &lab_name).await?.is_none(),
            "the lab record is left"
        );
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut first, &server, &token, &lab_name, &checks).await;
    with_cleanup(checks, cleanup)
}

/// M7.1: a lab folder with no record could not be removed through the API.
#[tokio::test]
#[ignore = "requires root, to write the server's labs folder"]
async fn lab_clean_002_clean_removes_a_lab_folder_with_no_record() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let admin_token = ws.login_admin().await?;
    let lab_id = format!("tf{:06}", std::process::id() % 1_000_000);
    let lab_dir = std::path::Path::new(SHERPA_LABS_PATH).join(&lab_id);
    fs::create_dir_all(&lab_dir)?;
    fs::write(lab_dir.join("sherpa_ssh_key"), "left by a destroy")?;

    let response = ws
        .rpc_call("clean", json!({ "token": admin_token, "lab_id": lab_id }))
        .await?;

    let left = lab_dir.exists();
    if left {
        fs::remove_dir_all(&lab_dir)?;
    }
    ensure!(
        response.get("error").is_none() && response["result"]["success"] == json!(true),
        "clean failed: {response}"
    );
    ensure!(!left, "clean left the lab folder");
    Ok(())
}

/// Concurrent `up` requests must get distinct subnets.
#[tokio::test]
#[ignore = "requires Docker and host network administration rights (CAP_NET_ADMIN)"]
async fn lab_concurrency_003_labs_brought_up_at_once_get_distinct_management_subnets() -> Result<()>
{
    let server = TestServer::start().await?;
    let mut first = TestWsClient::connect(&server).await?;
    let mut second = TestWsClient::connect(&server).await?;
    let token = first.login_lab_admin(&server).await?;
    bootstrap_images(&mut first, &token).await?;

    // lab_id must be exactly 8 characters
    let pid = std::process::id() % 1_000_000;
    let first_lab = format!("ca{pid:06}");
    let second_lab = format!("cb{pid:06}");
    let first_manifest = container_manifest(&first_lab);
    let second_manifest = container_manifest(&second_lab);

    let (first_up, second_up) = tokio::join!(
        lab_up(
            &mut first,
            &token,
            &first_lab,
            &first_manifest,
            Duration::from_secs(300)
        ),
        lab_up(
            &mut second,
            &token,
            &second_lab,
            &second_manifest,
            Duration::from_secs(300)
        ),
    );
    let checks: Result<()> = async {
        first_up.context("First concurrent lab up failed")?;
        second_up.context("Second concurrent lab up failed")?;
        let first_record = db::get_lab(&server.db, &first_lab).await?;
        let second_record = db::get_lab(&server.db, &second_lab).await?;
        ensure!(
            first_record.management_network != second_record.management_network,
            "Concurrent labs share management network {}",
            first_record.management_network
        );
        ensure!(
            first_record.management_network_v6 != second_record.management_network_v6,
            "Concurrent labs share IPv6 management network"
        );
        Ok(())
    }
    .await;
    let first_cleanup = cleanup_lab(&mut first, &server, &token, &first_lab, &checks).await;
    let second_cleanup = cleanup_lab(&mut first, &server, &token, &second_lab, &checks).await;
    with_cleanup(with_cleanup(checks, first_cleanup), second_cleanup)
}

// ── VM Lab Lifecycle ──

/// Fail unless the host can run VMs: a missing requirement is a failure, not a pass.
fn require_kvm() -> Result<()> {
    ensure!(
        std::path::Path::new("/dev/kvm").exists(),
        "/dev/kvm is required for the VM lab tests"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires KVM, libvirt with the alpine_linux image, Docker and root"]
async fn lab_up_002_a_vm_lab_comes_up_and_its_node_accepts_an_ssh_login() -> Result<()> {
    require_kvm()?;
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    bootstrap_images(&mut ws, &token).await?;

    // lab_id must be exactly 8 characters
    let lab_name = format!("vu{:06}", std::process::id() % 1_000_000);
    let manifest = vm_manifest(&lab_name);

    let checks: Result<()> = async {
        lab_up(&mut ws, &token, &lab_name, &manifest, VM_TIMEOUT).await?;
        let address = node_address(&server, &lab_name, "vm1").await?;
        // Readiness waits for the SSH port; cloud-init may still be writing the account.
        ensure!(
            wait_for_password_login(&address, TEST_LAB_ADMIN, TEST_DEVICE_PASSWORD, VM_TIMEOUT)
                .await?,
            "the VM did not take the owner's SSH login"
        );
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab_name, &checks).await;
    with_cleanup(checks, cleanup)
}

#[tokio::test]
#[ignore = "requires KVM, libvirt with the alpine_linux image, Docker and root"]
async fn lab_destroy_002_a_vm_lab_is_destroyed_with_nothing_left_in_libvirt() -> Result<()> {
    require_kvm()?;
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    bootstrap_images(&mut ws, &token).await?;

    // lab_id must be exactly 8 characters
    let lab_name = format!("vd{:06}", std::process::id() % 1_000_000);
    let manifest = vm_manifest(&lab_name);

    let checks: Result<()> = async {
        lab_up(&mut ws, &token, &lab_name, &manifest, VM_TIMEOUT).await?;
        ensure!(
            !libvirt_leftovers(&lab_name)?.is_empty(),
            "up created nothing in libvirt to destroy"
        );
        let (_s, response) = ws
            .rpc_call_streaming(
                "destroy",
                json!({ "token": token, "lab_id": lab_name }),
                VM_TIMEOUT,
            )
            .await?;
        ensure!(
            response.get("error").is_none(),
            "destroy should succeed: {response}"
        );
        let leftovers = libvirt_leftovers(&lab_name)?;
        ensure!(
            leftovers.is_empty(),
            "destroy left these in libvirt: {leftovers:?}"
        );
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab_name, &checks).await;
    with_cleanup(checks, cleanup)
}

// ── Error Cases ──

#[tokio::test]
async fn lab_preflight_001_a_manifest_naming_an_image_that_is_not_imported_is_refused_naming_the_image()
-> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;

    // Use a model that doesn't have images imported
    let manifest = json!({
        "name": "test-noimg",
        "nodes": [
            {
                "name": "bad1",
                "model": "ubuntu_linux",
            }
        ]
    });

    let (_s, response) = ws
        .rpc_call_streaming(
            "up",
            json!({
                "token": token,
                "lab_id": "noimg001",
                "manifest": manifest,
            }),
            Duration::from_secs(30),
        )
        .await?;

    assert!(
        response["error"].to_string().contains(
            "Node config not found for model: ubuntu_linux kind: virtual_machine (node: bad1)"
        ),
        "lab up with missing image should fail: {response:?}"
    );
    assert!(
        db::find_lab(&server.db, "noimg001").await?.is_none(),
        "a refused up created a lab record"
    );

    Ok(())
}

#[tokio::test]
async fn lab_preflight_002_an_invalid_manifest_is_refused() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;

    // Empty manifest - missing required fields
    let (_s, response) = ws
        .rpc_call_streaming(
            "up",
            json!({
                "token": token,
                "lab_id": "badmf001",
                "manifest": {},
            }),
            Duration::from_secs(10),
        )
        .await?;

    assert!(
        response["error"]["context"]
            .as_str()
            .is_some_and(|context| context.contains("missing field `name`")),
        "invalid manifest should fail: {response:?}"
    );

    Ok(())
}

#[tokio::test]
async fn access_control_001_an_unauthenticated_up_is_refused() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    let (_s, response) = ws
        .rpc_call_streaming(
            "up",
            json!({
                "lab_id": "noauth01",
                "manifest": {"name": "test", "nodes": []},
            }),
            Duration::from_secs(10),
        )
        .await?;

    assert_eq!(
        response["error"]["code"],
        json!(RpcErrorCode::AuthRequired),
        "up without auth should fail: {response}"
    );

    Ok(())
}

#[tokio::test]
async fn lab_preflight_003_a_lab_id_already_in_use_is_refused() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;

    // Seed a lab record directly in the DB to simulate an already-running lab
    let admin_user = db::get_user(&server.db, "admin").await?;
    let lab_id = "dupetest";
    db::create_lab(
        &server.db,
        "duplicate-test-lab",
        lab_id,
        &admin_user,
        &ipv4_lab_networks("10.0.1.0/24", "10.0.1.1", "10.0.1.254"),
        &db::LabDetails::default(),
    )
    .await?;

    // Attempt to `up` a lab with the same lab_id — should be rejected
    let manifest = json!({
        "name": "duplicate-test-lab",
        "nodes": [
            {
                "name": "srl1",
                "model": "nokia_srlinux",
            }
        ]
    });

    let (_statuses, response) = ws
        .rpc_call_streaming(
            "up",
            json!({
                "token": token,
                "lab_id": lab_id,
                "manifest": manifest,
            }),
            Duration::from_secs(10),
        )
        .await?;

    // Should get an RPC error
    let error = response
        .get("error")
        .expect("Expected RPC error for duplicate lab_id");
    let error_message = error.get("message").and_then(|m| m.as_str()).unwrap_or("");

    assert!(
        error_message.contains("already exists") || error_message.contains("already uses"),
        "Expected lab-already-exists error, got: {error_message}"
    );
    // M3.1: the refused `up` must not roll back the lab that holds the ID.
    assert!(
        db::find_lab(&server.db, lab_id).await?.is_some(),
        "the refused up removed the existing lab"
    );

    Ok(())
}

/// A user with the test SSH key, which redeploy requires of the lab owner.
async fn create_keyed_user(server: &TestServer, name: &str, is_admin: bool) -> Result<DbUser> {
    let user = db::create_user(
        &server.db,
        name.to_owned(),
        TEST_ADMIN_PASSWORD,
        is_admin,
        vec![TEST_SSH_PUBLIC_KEY.to_owned()],
    )
    .await?;
    db::set_device_password(&server.db, &db::get_user_id(&user)?, TEST_DEVICE_PASSWORD).await?;
    Ok(user)
}

/// RPC redeploy as `username`: the owner reaches the node lookup, anyone else is refused
/// before any progress is streamed.
async fn check_redeploy_rpc(
    ws: &mut TestWsClient,
    lab_id: &str,
    owner: &str,
    username: &str,
    token: &str,
) -> Result<()> {
    let owns_lab = username == owner;
    let (statuses, response) = ws.rpc_call_streaming("redeploy", json!({
        "token": token, "lab_id": lab_id, "node_name": "vm",
        "username": owner,
        "manifest": { "name": "auth-test", "nodes": [{ "name": "vm", "model": "ubuntu_linux" }] },
    }), Duration::from_secs(30)).await?;
    let expected = if owns_lab {
        RpcErrorCode::ServerError
    } else {
        RpcErrorCode::AccessDenied
    };
    ensure!(
        response["error"]["code"] == json!(expected),
        "Wrong RPC authorization for {username}: {response}"
    );
    if owns_lab {
        ensure!(
            response["error"]["context"]
                .as_str()
                .is_some_and(|message| message.contains("Node 'vm' not found")),
            "Owner did not reach node lookup: {response}"
        );
    } else {
        ensure!(
            statuses.is_empty(),
            "Unauthorized request started streaming node operations"
        );
    }
    Ok(())
}

/// REST and cookie redeploy as `username`: the owner gets past authorization to the missing
/// manifest, anyone else gets 403.
async fn check_redeploy_http(
    http: &Client,
    server: &TestServer,
    lab_id: &str,
    owner: &str,
    username: &str,
    token: &str,
) -> Result<()> {
    let owns_lab = username == owner;
    // Missing manifest yields 400 only after owner authorization passes.
    let response = http
        .post(format!(
            "http://{}/api/v1/labs/{lab_id}/nodes/vm/redeploy",
            server.addr
        ))
        .bearer_auth(token)
        .json(&json!({ "username": owner }))
        .send()
        .await?;
    ensure!(
        response.status()
            == if owns_lab {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::FORBIDDEN
            },
        "Wrong HTTP authorization for {username}: {}",
        response.status()
    );
    // An authorized cookie request is refused: the lab has no stored definition.
    let response = http
        .post(format!(
            "http://{}/labs/{lab_id}/nodes/vm/redeploy",
            server.addr
        ))
        .header(
            reqwest::header::COOKIE,
            format!("{AUTH_COOKIE_NAME}={token}"),
        )
        .send()
        .await?;
    ensure!(
        response.status()
            == if owns_lab {
                StatusCode::CONFLICT
            } else {
                StatusCode::FORBIDDEN
            },
        "Wrong cookie authorization for {username}: {}",
        response.status()
    );
    Ok(())
}

/// Database and local API sockets only; no VM, container or lab files are created.
#[tokio::test]
async fn access_control_002_only_the_labs_owner_can_redeploy_over_rpc_and_http() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    // A personal admin: the built-in `admin` account cannot redeploy at all.
    let admin = create_keyed_user(&server, "dana", true).await?;
    let admin_token = ws.login("dana", TEST_ADMIN_PASSWORD).await?;
    let alice = create_keyed_user(&server, "alice", false).await?;
    let bob = create_keyed_user(&server, "bob", false).await?;
    let alice_token = ws.login("alice", TEST_ADMIN_PASSWORD).await?;
    let bob_token = ws.login("bob", TEST_ADMIN_PASSWORD).await?;
    let http = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let callers = [
        ("alice", alice_token.as_str()),
        ("bob", bob_token.as_str()),
        ("dana", admin_token.as_str()),
    ];
    // Lab ownership is immutable, so exercise both regular-user and admin-owned labs.
    for (index, owner) in [&alice, &bob, &admin].into_iter().enumerate() {
        let lab_id = format!("{:08x}", rand::random::<u32>());
        let lab = db::create_lab(
            &server.db,
            "redeploy-auth",
            &lab_id,
            owner,
            &ipv4_lab_networks(
                &format!("10.200.{index}.0/24"),
                &format!("10.200.{index}.1"),
                &format!("10.200.{index}.2"),
            ),
            &db::LabDetails::default(),
        )
        .await?;
        // As `up` leaves it once built: a redeploy needs a ready lab.
        db::update_lab_phase_from(&server.db, &lab_id, LabPhase::Creating, LabPhase::Ready).await?;
        for (username, token) in callers {
            check_redeploy_rpc(&mut ws, &lab_id, &owner.username, username, token).await?;
            check_redeploy_http(&http, &server, &lab_id, &owner.username, username, token).await?;
        }
        ensure!(
            db::list_nodes_by_lab(&server.db, db::get_lab_id(&lab)?)
                .await?
                .is_empty(),
            "Authorization checks created node resources"
        );
        db::delete_lab_from(&server.db, &lab_id, LabPhase::Ready).await?;
    }
    Ok(())
}
