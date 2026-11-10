use anyhow::{Context, Result, bail, ensure};
use bollard::Docker;
use bollard::query_parameters::{InspectContainerOptions, ListNetworksOptions};
use db::DefaultRequest;
use jato::{Auth, ConnectOptions, Exit, Hop, HostKeyPolicy, Node, Session};
use reqwest::{Client, StatusCode};
use serde_json::json;
use shared::data::{
    DbUser, LabNodeActionResponse, LabPhase, LabState, NodeConfig, NodeModel, NodeState,
};
use shared::error::RpcErrorCode;
use shared::konst::{SHERPA_LABS_PATH, SSH_PORT};
use sherpad::auth::cookies::AUTH_COOKIE_NAME;
use std::collections::BTreeMap;
use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::helpers::test_server::{
    TEST_ADMIN_PASSWORD, TEST_SSH_PUBLIC_KEY, TestServer, test_config,
};
use crate::helpers::ws_client::TestWsClient;

/// Checks a personal account's user, home, shell and passwordless sudo.
const ACCOUNT_CHECK: &str = r#"test "$(id -un)" = "${USER}" && test "$HOME" = "/home/${USER}" && test "$SHELL" = /bin/bash && test -d "$HOME" && sudo -n true"#;

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

/// Helper to create a minimal VM manifest
fn vm_manifest(lab_name: &str) -> serde_json::Value {
    json!({
        "name": lab_name,
        "nodes": [
            {
                "name": "vm1",
                "model": "ubuntu_linux",
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

/// IPv4-only lab networks for tests that seed lab records directly.
fn ipv4_lab_networks(
    loopback_network: &str,
    management_network: &str,
    gateway_ipv4: &str,
    router_ipv4: &str,
) -> db::LabNetworks {
    db::LabNetworks {
        loopback_network: loopback_network.to_string(),
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

/// Wait until a guest accepts TCP connections on its SSH port.
async fn wait_for_ssh(address: &str, timeout: Duration) -> Result<()> {
    let target: SocketAddr = format!("{address}:22").parse()?;
    let start = Instant::now();
    while start.elapsed() < timeout {
        if TcpStream::connect_timeout(&target, Duration::from_secs(5)).is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    bail!("{address} did not accept SSH within {}s", timeout.as_secs())
}

/// Log in with one key and check the personal account's shell, home and passwordless sudo.
/// A connection or login that fails counts as `false`, as the guest may still be provisioning.
async fn ssh_login(destination: &str, key: &str) -> Result<bool> {
    let Some((user, host)) = destination.split_once('@') else {
        bail!("SSH destination '{destination}' is not user@host");
    };
    let hop = Hop::new(
        Node::new(host, SSH_PORT),
        Auth::new(user).key_file(key, None),
        HostKeyPolicy::Insecure,
    );
    let mut options = ConnectOptions::default();
    options.connect_timeout = Duration::from_secs(10);
    let session = match Session::connect(&hop, &options).await {
        Ok(session) => session,
        Err(error) => {
            eprintln!("SSH login to {destination} failed: {error}");
            return Ok(false);
        }
    };
    let output = session
        .run(ACCOUNT_CHECK, b"", Duration::from_secs(30))
        .await
        .context("Failed to run SSH acceptance check")?;
    if let Err(error) = session.close().await {
        eprintln!("SSH session to {destination} did not close cleanly: {error}");
    }
    Ok(output.exit == Exit::Code(0))
}

/// Retry a login that should succeed until the guest has finished provisioning it.
async fn wait_for_login(destination: &str, key: &str, timeout: Duration) -> Result<bool> {
    let start = Instant::now();
    loop {
        if ssh_login(destination, key).await? {
            return Ok(true);
        }
        if start.elapsed() >= timeout {
            return Ok(false);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
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
async fn a_container_that_exits_at_once_fails_its_node_not_the_lab() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    bootstrap_images(&mut ws, &token).await?;

    // generic_container keeps the manifest's commands; alpine is in the dev server's Docker.
    let mut alpine = NodeConfig::get_model(NodeModel::GenericContainer);
    alpine.repo = Some("alpine".to_owned());
    alpine.version = "latest".to_owned();
    alpine.default = true;
    db::upsert_node_image(&server.db, alpine, DefaultRequest::Keep).await?;

    let lab_name = format!("tx{:06}", std::process::id() % 1_000_000);
    let manifest = json!({
        "name": lab_name,
        "nodes": [
            { "name": "dead1", "model": "generic_container", "commands": ["/bin/false"], "data_interface_count": 0 },
            { "name": "live1", "model": "generic_container", "commands": ["sleep", "600"], "data_interface_count": 0 },
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

#[tokio::test]
#[ignore = "requires Docker and host network administration rights (CAP_NET_ADMIN)"]
async fn admin_clean_removes_a_running_lab() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    let admin_token = ws.login_admin().await?;
    bootstrap_images(&mut ws, &token).await?;

    // generic_container keeps the manifest's commands; alpine is in the dev server's Docker.
    let mut alpine = NodeConfig::get_model(NodeModel::GenericContainer);
    alpine.repo = Some("alpine".to_owned());
    alpine.version = "latest".to_owned();
    alpine.default = true;
    db::upsert_node_image(&server.db, alpine, DefaultRequest::Keep).await?;

    let lab_name = format!("tk{:06}", std::process::id() % 1_000_000);
    let manifest = json!({
        "name": lab_name,
        "nodes": [
            { "name": "box1", "model": "generic_container", "commands": ["sleep", "600"], "data_interface_count": 0 },
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
#[ignore = "requires Docker and host network administration rights (CAP_NET_ADMIN)"]
async fn a_redeploy_that_fails_after_teardown_fails_its_node() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    bootstrap_images(&mut ws, &token).await?;

    // generic_container keeps the manifest's commands; alpine is in the dev server's Docker.
    let mut alpine = NodeConfig::get_model(NodeModel::GenericContainer);
    alpine.repo = Some("alpine".to_owned());
    alpine.version = "latest".to_owned();
    alpine.default = true;
    db::upsert_node_image(&server.db, alpine, DefaultRequest::Keep).await?;

    let lab_name = format!("tf{:06}", std::process::id() % 1_000_000);
    let manifest = json!({
        "name": lab_name,
        "nodes": [
            { "name": "box1", "model": "generic_container", "commands": ["sleep", "600"], "data_interface_count": 0 },
        ]
    });
    // Docker creates the container, then cannot start it: the node is already torn down.
    let broken = json!({
        "name": lab_name,
        "nodes": [
            { "name": "box1", "model": "generic_container", "commands": ["sleep", "600"], "data_interface_count": 0, "user": "nosuchuser" },
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

#[tokio::test]
#[ignore = "requires Docker and host network administration rights (CAP_NET_ADMIN)"]
async fn the_scanner_records_what_docker_reports_but_not_over_up() -> Result<()> {
    let server = TestServer::start_with_scanner(1).await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    bootstrap_images(&mut ws, &token).await?;

    // generic_container keeps the manifest's commands; alpine is in the dev server's Docker.
    let mut alpine = NodeConfig::get_model(NodeModel::GenericContainer);
    alpine.repo = Some("alpine".to_owned());
    alpine.version = "latest".to_owned();
    alpine.default = true;
    db::upsert_node_image(&server.db, alpine, DefaultRequest::Keep).await?;

    let lab_name = format!("sc{:06}", std::process::id() % 1_000_000);
    let manifest = json!({
        "name": lab_name,
        "nodes": [
            { "name": "live1", "model": "generic_container", "commands": ["sleep", "600"], "data_interface_count": 0 },
            { "name": "live2", "model": "generic_container", "commands": ["sleep", "600"], "data_interface_count": 0 },
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
async fn test_container_lab_up_and_inspect() -> Result<()> {
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
async fn test_container_lab_down_and_resume() -> Result<()> {
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
async fn test_container_lab_destroy() -> Result<()> {
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

/// Add the alpine `generic_container` image, which keeps the manifest's commands, and return a
/// one-node manifest for lab `lab_name` that sleeps.
async fn sleeping_container_lab(server: &TestServer, lab_name: &str) -> Result<serde_json::Value> {
    let mut alpine = NodeConfig::get_model(NodeModel::GenericContainer);
    alpine.repo = Some("alpine".to_owned());
    "latest".clone_into(&mut alpine.version);
    alpine.default = true;
    db::upsert_node_image(&server.db, alpine, DefaultRequest::Keep).await?;
    Ok(json!({
        "name": lab_name,
        "nodes": [
            { "name": "box1", "model": "generic_container", "commands": ["sleep", "600"], "data_interface_count": 0 },
        ]
    }))
}

/// M3.1: a second `up` of the same lab used to fail in phase 3 and roll back the first one's
/// lab. Now it is refused at the reservation and touches nothing.
#[tokio::test]
#[ignore = "requires Docker and host network administration rights (CAP_NET_ADMIN)"]
async fn two_ups_of_one_lab_leave_the_first_alone() -> Result<()> {
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
async fn two_destroys_of_one_lab_both_succeed() -> Result<()> {
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
async fn admin_clean_removes_a_lab_folder_with_no_record() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let admin_token = ws.login_admin().await?;
    let lab_id = format!("tf{:06}", std::process::id() % 1_000_000);
    let lab_dir = std::path::Path::new(SHERPA_LABS_PATH).join(&lab_id);
    fs::create_dir_all(&lab_dir)?;
    fs::write(lab_dir.join("lab-info.toml"), "left by a destroy")?;

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
async fn test_concurrent_lab_up_allocates_distinct_subnets() -> Result<()> {
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
            first_record.loopback_network != second_record.loopback_network,
            "Concurrent labs share loopback network {}",
            first_record.loopback_network
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

#[tokio::test]
#[ignore = "requires KVM, libvirt and the ubuntu_linux image"]
async fn test_vm_lab_up_and_destroy() -> Result<()> {
    // A missing requirement is a failure, not a pass: this test must actually run.
    ensure!(
        std::path::Path::new("/dev/kvm").exists(),
        "/dev/kvm is required for the VM lab test"
    );

    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    bootstrap_images(&mut ws, &token).await?;

    // lab_id must be exactly 8 characters
    let lab_name = format!("vm{:06}", std::process::id() % 1_000_000);
    let manifest = vm_manifest(&lab_name);

    let checks: Result<()> = async {
        let statuses = lab_up(
            &mut ws,
            &token,
            &lab_name,
            &manifest,
            Duration::from_secs(300),
        )
        .await?;
        ensure!(
            !statuses.is_empty(),
            "should have status messages during up"
        );
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab_name, &checks).await;
    with_cleanup(checks, cleanup)
}

// ── Error Cases ──

#[tokio::test]
async fn test_lab_up_missing_image() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;

    // Use a model that doesn't have images imported
    let manifest = json!({
        "name": "test-noimg",
        "nodes": [
            {
                "name": "bad1",
                "model": "juniper_vsrx",
            }
        ]
    });

    let (_s, response) = ws
        .rpc_call_streaming(
            "up",
            json!({
                "token": token,
                "lab_id": "test-noimg",
                "manifest": manifest,
            }),
            Duration::from_secs(30),
        )
        .await?;

    assert!(
        response.get("error").is_some(),
        "lab up with missing image should fail: {response:?}"
    );

    Ok(())
}

#[tokio::test]
async fn test_lab_up_invalid_manifest() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;

    // Empty manifest - missing required fields
    let (_s, response) = ws
        .rpc_call_streaming(
            "up",
            json!({
                "token": token,
                "lab_id": "test-invalid",
                "manifest": {},
            }),
            Duration::from_secs(10),
        )
        .await?;

    assert!(
        response.get("error").is_some(),
        "invalid manifest should fail: {response:?}"
    );

    Ok(())
}

#[tokio::test]
async fn test_lab_up_requires_auth() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    let (_s, response) = ws
        .rpc_call_streaming(
            "up",
            json!({
                "lab_id": "test-noauth",
                "manifest": {"name": "test", "nodes": []},
            }),
            Duration::from_secs(10),
        )
        .await?;

    assert!(
        response.get("error").is_some(),
        "up without auth should fail"
    );

    Ok(())
}

#[tokio::test]
async fn test_lab_up_rejects_duplicate_lab_id() -> Result<()> {
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
        &ipv4_lab_networks("10.0.0.0/24", "10.0.1.0/24", "10.0.1.1", "10.0.1.254"),
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

/// Run only on a disposable Sherpa host; see tests/specs/integration/current-user.md.
#[tokio::test]
#[ignore = "requires Linux VM image, host infrastructure and two explicit SSH key pairs"]
async fn current_user_vm_login_sudo_resume_and_owner_redeploy() -> Result<()> {
    let private_key = std::env::var("SHERPA_GUEST_TEST_KEY")
        .context("Set SHERPA_GUEST_TEST_KEY to the original private key path")?;
    let rotated_key = std::env::var("SHERPA_GUEST_TEST_ROTATED_KEY")
        .context("Set SHERPA_GUEST_TEST_ROTATED_KEY to a different private key path")?;
    let public_key = fs::read_to_string(format!("{private_key}.pub"))?;
    let rotated_public_key = fs::read_to_string(format!("{rotated_key}.pub"))?;
    ensure!(
        public_key.split_whitespace().nth(1) != rotated_public_key.split_whitespace().nth(1),
        "Use distinct SSH key pairs"
    );
    let model =
        std::env::var("SHERPA_GUEST_TEST_MODEL").unwrap_or_else(|_| "ubuntu_linux".to_owned());
    let mut config = test_config()?;
    if let Ok(images_dir) = std::env::var("SHERPA_GUEST_TEST_IMAGES_DIR") {
        config.images_dir = images_dir;
    }
    let server = TestServer::start_with_config(config).await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let admin_token = ws.login_admin().await?;
    bootstrap_images(&mut ws, &admin_token).await?;
    let username = format!("guest{}", std::process::id());
    let mut owner = db::create_user(
        &server.db,
        username.clone(),
        TEST_ADMIN_PASSWORD,
        false,
        vec![public_key.trim().to_owned()],
    )
    .await?;
    let owner_token = ws.login(&username, TEST_ADMIN_PASSWORD).await?;
    let lab_id = format!("{:08x}", rand::random::<u32>());
    ensure!(
        db::get_lab(&server.db, &lab_id).await.is_err(),
        "Acceptance lab ID is already in use"
    );
    let manifest = json!({ "name": lab_id, "provision_current_user": true, "nodes": [{ "name": "vm", "model": model }] });
    let timeout = Duration::from_mins(15);

    // Return failures as errors so cleanup still runs when any check fails.
    let checks: Result<()> = async {
        let mut no_keys = owner.clone();
        no_keys.ssh_keys.clear();
        // `update_user` refuses a stale read, so each restore carries the version just written.
        owner.updated_at = db::update_user(&server.db, no_keys).await?.updated_at;
        let (_, rejected) = ws.rpc_call_streaming("up", json!({ "token": owner_token, "lab_id": lab_id, "manifest": manifest }), timeout).await?;
        ensure!(rejected.get("error").is_some(), "Startup accepted an owner without keys");
        ensure!(db::get_lab(&server.db, &lab_id).await.is_err(), "Rejected startup created a lab record");
        owner = db::update_user(&server.db, owner.clone()).await?;
        let (_, response) = ws.rpc_call_streaming("up", json!({ "token": owner_token, "lab_id": lab_id, "manifest": manifest, "username": "admin" }), timeout).await?;
        ensure!(response.get("error").is_none(), "Startup failed: {response}");
        ensure!(response["result"]["lab_info"]["user"] == username, "Client-spoofed username selected lab owner");
        ensure!(response["result"]["lab_info"]["ssh_usernames"][0] == json!({"node_name": "vm", "username": username}), "Personal SSH metadata missing");
        let lab = db::get_lab(&server.db, &lab_id).await?;
        ensure!(Some(lab.user.clone()) == owner.id, "Database owner differs from authenticated creator");
        let node = db::get_node_by_name_and_lab(&server.db, "vm", db::get_lab_id(&lab)?).await?;
        let ip = node.mgmt_ipv4.context("VM management address missing")?;
        let destination = format!("{username}@{ip}");

        // Readiness only waits for the SSH port; cloud-init may still be writing accounts.
        ensure!(wait_for_login(&destination, &private_key, timeout).await?, "Personal login/home/Bash/passwordless sudo failed");
        let (_, admin_rejected) = ws.rpc_call_streaming("redeploy", json!({ "token": admin_token, "lab_id": lab_id, "node_name": "vm", "manifest": manifest, "username": username }), timeout).await?;
        ensure!(admin_rejected["error"]["code"] == json!(RpcErrorCode::AccessDenied), "Non-owner admin was allowed to redeploy");
        ensure!(ssh_login(&destination, &private_key).await?, "Denied admin redeploy affected the running guest");
        let mut no_keys = owner.clone();
        no_keys.ssh_keys.clear();
        owner.updated_at = db::update_user(&server.db, no_keys).await?.updated_at;
        let (_, rejected) = ws.rpc_call_streaming("redeploy", json!({ "token": owner_token, "lab_id": lab_id, "node_name": "vm", "manifest": manifest }), timeout).await?;
        ensure!(rejected.get("error").is_some(), "Missing keys were accepted during redeploy");
        ensure!(ssh_login(&destination, &private_key).await?, "Rejected redeploy tore down existing guest");

        owner.ssh_keys = vec![rotated_public_key.trim().to_owned()];
        db::update_user(&server.db, owner.clone()).await?;
        let (_, stopped) = ws.rpc_call_streaming("down", json!({ "token": owner_token, "lab_id": lab_id }), timeout).await?;
        ensure!(stopped.get("error").is_none(), "down failed: {stopped}");
        let (statuses, resumed) = ws.rpc_call_streaming("resume", json!({ "token": owner_token, "lab_id": lab_id }), timeout).await?;
        ensure!(resumed.get("error").is_none(), "resume failed: {resumed}");
        // Resume waits for the cold-booted VM, as `up` does, and streams its progress.
        ensure!(resumed["result"]["results"][0]["message"] == "Started and ready", "Resume did not wait for readiness: {resumed}");
        ensure!(!statuses.is_empty(), "Resume streamed no readiness progress");
        // Distinguish an unbooted guest from a key change before checking logins.
        wait_for_ssh(&ip, timeout).await.context("Resumed VM did not accept SSH")?;
        ensure!(ssh_login(&destination, &private_key).await?, "Resume replaced existing guest keys");
        ensure!(!ssh_login(&destination, &rotated_key).await?, "Profile key was synchronized during resume");

        let (_, redeployed) = ws.rpc_call_streaming("redeploy", json!({ "token": owner_token, "lab_id": lab_id, "node_name": "vm", "manifest": manifest }), timeout).await?;
        ensure!(redeployed.get("error").is_none(), "Owner redeploy failed: {redeployed}");
        ensure!(wait_for_login(&destination, &rotated_key, timeout).await?, "Redeploy did not provision owner's current keys");
        ensure!(!ssh_login(&destination, &private_key).await?, "Redeploy retained removed owner key");
        let downloaded = ws.rpc_call("download", json!({ "token": admin_token, "lab_id": lab_id })).await?;
        ensure!(downloaded.get("error").is_none(), "Admin configuration refresh failed: {downloaded}");
        ensure!(downloaded["result"]["lab_info"]["ssh_usernames"][0] == json!({"node_name": "vm", "username": username}), "Refreshed configuration selected administrator");
        let config = downloaded["result"]["ssh_config"].as_str().context("SSH config missing")?;
        ensure!(config.contains(&format!("User {username}")) && !config.contains("IdentityFile") && !config.contains("IdentitiesOnly"), "Personal SSH config restricts local keys");
        Ok(())
    }.await;
    let cleanup = cleanup_lab(&mut ws, &server, &owner_token, &lab_id, &checks).await;
    with_cleanup(checks, cleanup)
}

/// A user with the test SSH key, which redeploy requires of the lab owner.
async fn create_keyed_user(server: &TestServer, name: &str, is_admin: bool) -> Result<DbUser> {
    db::create_user(
        &server.db,
        name.to_owned(),
        TEST_ADMIN_PASSWORD,
        is_admin,
        vec![TEST_SSH_PUBLIC_KEY.to_owned()],
    )
    .await
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
    // An authorized cookie request stops at the absent saved manifest.
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
                StatusCode::INTERNAL_SERVER_ERROR
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
async fn redeploy_only_allows_current_owner_on_all_transports() -> Result<()> {
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
                &format!("127.127.{index}.0/24"),
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
