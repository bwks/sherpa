mod helpers;

use anyhow::{Context, Result, bail, ensure};
use reqwest::{Client, StatusCode};
use serde_json::json;
use shared::error::RpcErrorCode;
use sherpad::auth::cookies::AUTH_COOKIE_NAME;
use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::process::Command;
use std::time::{Duration, Instant};

use helpers::test_server::{TEST_ADMIN_PASSWORD, TestServer};
use helpers::ws_client::TestWsClient;

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
        "image.scan failed: {:?}",
        response
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
/// `token` must belong to the lab owner: the destroy service only lets owners destroy.
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
fn ssh_login(destination: &str, key: &str) -> Result<bool> {
    let status = Command::new("ssh")
        .args([
            "-F", "/dev/null", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-o",
            "IdentitiesOnly=yes", "-o", "IdentityAgent=none", "-o", "StrictHostKeyChecking=no",
            "-o", "UserKnownHostsFile=/dev/null", "-i", key, destination,
            "test \"$(id -un)\" = \"${USER}\" && test \"$HOME\" = \"/home/${USER}\" && test \"$SHELL\" = /bin/bash && test -d \"$HOME\" && sudo -n true",
        ])
        .status()
        .context("Failed to run SSH acceptance check")?;
    Ok(status.success())
}

/// Retry a login that should succeed until the guest has finished provisioning it.
async fn wait_for_login(destination: &str, key: &str, timeout: Duration) -> Result<bool> {
    let start = Instant::now();
    loop {
        if ssh_login(destination, key)? {
            return Ok(true);
        }
        if start.elapsed() >= timeout {
            return Ok(false);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
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
#[ignore]
async fn test_container_lab_up_and_inspect() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;
    bootstrap_images(&mut ws, &token).await?;

    // lab_id must be exactly 8 characters
    let lab_name = format!("tc{:06}", std::process::id() % 1_000_000);
    let manifest = container_manifest(&lab_name);

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
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab_name, &checks).await;
    with_cleanup(checks, cleanup)
}

#[tokio::test]
#[ignore]
async fn test_container_lab_down_and_resume() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;
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
        for method in ["down", "resume"] {
            let (_s, response) = ws
                .rpc_call_streaming(
                    method,
                    json!({ "token": token, "lab_id": lab_name }),
                    Duration::from_secs(300),
                )
                .await?;
            ensure!(
                response.get("error").is_none(),
                "{method} should succeed: {response}"
            );
        }
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab_name, &checks).await;
    with_cleanup(checks, cleanup)
}

#[tokio::test]
#[ignore]
async fn test_container_lab_destroy() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;
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

/// Concurrent `up` requests must get distinct subnets.
#[tokio::test]
#[ignore]
async fn test_concurrent_lab_up_allocates_distinct_subnets() -> Result<()> {
    let server = TestServer::start().await?;
    let mut first = TestWsClient::connect(&server).await?;
    let mut second = TestWsClient::connect(&server).await?;
    let token = first.login_admin().await?;
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
    let token = ws.login_admin().await?;
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
#[ignore]
async fn test_lab_up_missing_image() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

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
        "lab up with missing image should fail: {:?}",
        response
    );

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_lab_up_invalid_manifest() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

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
        "invalid manifest should fail: {:?}",
        response
    );

    Ok(())
}

#[tokio::test]
#[ignore]
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
#[ignore]
async fn test_lab_up_rejects_duplicate_lab_id() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // Seed a lab record directly in the DB to simulate an already-running lab
    let admin_user = db::get_user(&server.db, "admin").await?;
    let lab_id = "dupetest";
    db::create_lab(
        &server.db,
        "duplicate-test-lab",
        lab_id,
        &admin_user,
        &ipv4_lab_networks("10.0.0.0/24", "10.0.1.0/24", "10.0.1.1", "10.0.1.254"),
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
        error_message.contains("Lab already exists with status"),
        "Expected lab-already-exists error, got: {}",
        error_message
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
    let server =
        TestServer::start_with_config(std::env::var("SHERPA_GUEST_TEST_IMAGES_DIR").ok()).await?;
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
    let timeout = Duration::from_secs(900);

    // Return failures as errors so cleanup still runs when any check fails.
    let checks: Result<()> = async {
        let mut no_keys = owner.clone();
        no_keys.ssh_keys.clear();
        db::update_user(&server.db, no_keys).await?;
        let (_, rejected) = ws.rpc_call_streaming("up", json!({ "token": owner_token, "lab_id": lab_id, "manifest": manifest }), timeout).await?;
        ensure!(rejected.get("error").is_some(), "Startup accepted an owner without keys");
        ensure!(db::get_lab(&server.db, &lab_id).await.is_err(), "Rejected startup created a lab record");
        db::update_user(&server.db, owner.clone()).await?;
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
        ensure!(ssh_login(&destination, &private_key)?, "Denied admin redeploy affected the running guest");
        let mut no_keys = owner.clone();
        no_keys.ssh_keys.clear();
        db::update_user(&server.db, no_keys).await?;
        let (_, rejected) = ws.rpc_call_streaming("redeploy", json!({ "token": owner_token, "lab_id": lab_id, "node_name": "vm", "manifest": manifest }), timeout).await?;
        ensure!(rejected.get("error").is_some(), "Missing keys were accepted during redeploy");
        ensure!(ssh_login(&destination, &private_key)?, "Rejected redeploy tore down existing guest");

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
        ensure!(ssh_login(&destination, &private_key)?, "Resume replaced existing guest keys");
        ensure!(!ssh_login(&destination, &rotated_key)?, "Profile key was synchronized during resume");

        let (_, redeployed) = ws.rpc_call_streaming("redeploy", json!({ "token": owner_token, "lab_id": lab_id, "node_name": "vm", "manifest": manifest }), timeout).await?;
        ensure!(redeployed.get("error").is_none(), "Owner redeploy failed: {redeployed}");
        ensure!(wait_for_login(&destination, &rotated_key, timeout).await?, "Redeploy did not provision owner's current keys");
        ensure!(!ssh_login(&destination, &private_key)?, "Redeploy retained removed owner key");
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

/// Database and local API sockets only; no VM, container or lab files are created.
#[tokio::test]
#[ignore = "requires the disposable SurrealDB test database"]
async fn redeploy_only_allows_current_owner_on_all_transports() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let admin_token = ws.login_admin().await?;
    let admin = db::get_user(&server.db, "admin").await?;
    let alice = db::create_user(
        &server.db,
        "alice".to_owned(),
        TEST_ADMIN_PASSWORD,
        false,
        vec![],
    )
    .await?;
    let bob = db::create_user(
        &server.db,
        "bob".to_owned(),
        TEST_ADMIN_PASSWORD,
        false,
        vec![],
    )
    .await?;
    let alice_token = ws.login("alice", TEST_ADMIN_PASSWORD).await?;
    let bob_token = ws.login("bob", TEST_ADMIN_PASSWORD).await?;
    let http = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let callers = [
        ("alice", alice_token.as_str()),
        ("bob", bob_token.as_str()),
        ("admin", admin_token.as_str()),
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
        )
        .await?;
        for (username, token) in callers {
            let owns_lab = username == owner.username;
            let (statuses, response) = ws.rpc_call_streaming("redeploy", json!({
                "token": token, "lab_id": lab_id, "node_name": "vm",
                "username": owner.username,
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
            // Missing manifest yields 400 only after owner authorization passes.
            let response = http
                .post(format!(
                    "http://{}/api/v1/labs/{lab_id}/nodes/vm/redeploy",
                    server.addr
                ))
                .bearer_auth(token)
                .json(&json!({ "username": owner.username }))
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
        }
        ensure!(
            db::list_nodes_by_lab(&server.db, db::get_lab_id(&lab)?)
                .await?
                .is_empty(),
            "Authorization checks created node resources"
        );
        db::delete_lab(&server.db, &lab_id).await?;
    }
    Ok(())
}
