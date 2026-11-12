use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::time::Duration;

use shared::data::{NodeKind, NodeModel};

use crate::helpers::test_server::TestServer;
use crate::helpers::ws_client::TestWsClient;

// ── Image Scan ──

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn test_image_scan_discovers_images() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let (_statuses, response) = ws
        .rpc_call_streaming(
            "image.scan",
            json!({ "token": token }),
            Duration::from_secs(60),
        )
        .await?;

    assert!(
        response.get("result").is_some(),
        "image.scan should succeed: {response:?}"
    );
    // Status messages are emitted per discovered image; OK if none found on a clean env

    Ok(())
}

/// The status the scan in `response` gave `model` at `version`, if it found it.
fn scanned_status<'a>(response: &'a Value, model: &str, version: &str) -> Option<&'a str> {
    response["result"]["scanned"]
        .as_array()?
        .iter()
        .find(|image| image["model"] == model && image["version"] == version)?["status"]
        .as_str()
}

#[tokio::test]
#[ignore = "requires a Docker daemon and internet access to pull an image"]
async fn test_image_scan_records_a_local_image_for_a_model_and_repo() -> Result<()> {
    container::pull_image("busybox", "1.36", |_| {}).await?;
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;
    let scan = |dry_run: bool| {
        json!({
            "token": token,
            "kind": "container",
            "dry_run": dry_run,
            "model": "generic_container",
            "repo": "busybox",
        })
    };

    let dry_run = ws.rpc_call("image.scan", scan(true)).await?;

    assert_eq!(
        scanned_status(&dry_run, "generic_container", "1.36"),
        Some("would_import (default)"),
        "{dry_run:?}"
    );
    let unrecorded = db::find_node_image_by_model_kind_version(
        &server.db,
        &NodeModel::GenericContainer,
        &NodeKind::Container,
        "1.36",
    )
    .await?;
    assert!(unrecorded.is_none(), "a dry run records nothing");

    let scanned = ws.rpc_call("image.scan", scan(false)).await?;

    assert_eq!(
        scanned_status(&scanned, "generic_container", "1.36"),
        Some("imported (default)"),
        "{scanned:?}"
    );
    let record = db::find_node_image_by_model_kind_version(
        &server.db,
        &NodeModel::GenericContainer,
        &NodeKind::Container,
        "1.36",
    )
    .await?
    .context("the scan should record busybox:1.36 for generic_container")?;
    assert_eq!(record.repo.as_deref(), Some("busybox"));
    assert!(record.default);

    let rescanned = ws.rpc_call("image.scan", scan(false)).await?;

    assert_eq!(
        scanned_status(&rescanned, "generic_container", "1.36"),
        Some("already_exists"),
        "{rescanned:?}"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires a Docker daemon and internet access to pull an image"]
async fn test_image_scan_matches_a_built_in_repo_and_skips_models_without_one() -> Result<()> {
    container::pull_image("nats", "2.10-alpine", |_| {}).await?;
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let response = ws
        .rpc_call("image.scan", json!({ "token": token, "kind": "container" }))
        .await?;

    assert_eq!(
        scanned_status(&response, "nats_bus", "2.10-alpine").map(|s| s.starts_with("imported")),
        Some(true),
        "{response:?}"
    );
    let record = db::find_node_image_by_model_kind_version(
        &server.db,
        &NodeModel::NatsBus,
        &NodeKind::Container,
        "2.10-alpine",
    )
    .await?
    .context("the scan should record nats:2.10-alpine for nats_bus")?;
    assert_eq!(record.repo.as_deref(), Some("nats"));
    let skipped: Vec<&str> = response["result"]["skipped"]
        .as_array()
        .context("the scan should list skipped models")?
        .iter()
        .filter_map(|model| model["model"].as_str())
        .collect();
    assert_eq!(skipped, ["mysql_db", "postgresql_db", "generic_container"]);

    Ok(())
}

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn test_image_scan_refuses_a_repo_with_a_tag() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let response = ws
        .rpc_call(
            "image.scan",
            json!({
                "token": token,
                "kind": "container",
                "model": "mysql_db",
                "repo": "mysql:8.4",
            }),
        )
        .await?;

    let error = response["error"].to_string();
    assert!(error.contains("without a tag"), "{response:?}");

    Ok(())
}

// ── Image List ──

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn test_image_list_returns_images() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // First scan to register images
    let (_s, _r) = ws
        .rpc_call_streaming(
            "image.scan",
            json!({ "token": token }),
            Duration::from_secs(60),
        )
        .await?;

    // List all images
    let response = ws.rpc_call("image.list", json!({ "token": token })).await?;

    assert!(
        response.get("result").is_some(),
        "image.list should succeed: {response:?}"
    );

    Ok(())
}

// ── Image Show ──

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn test_image_show_returns_config() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // Scan first
    let (_s, _r) = ws
        .rpc_call_streaming(
            "image.scan",
            json!({ "token": token }),
            Duration::from_secs(60),
        )
        .await?;

    // Show a specific model — use ubuntu_linux which should be on disk
    let response = ws
        .rpc_call(
            "image.show",
            json!({
                "token": token,
                "model": "ubuntu_linux",
            }),
        )
        .await?;

    // If image was found, result should have config details
    // image.show returns ShowImageResponse { image: NodeConfig }
    if response.get("result").is_some() {
        let result = response.get("result").unwrap();
        assert!(
            result.get("image").is_some(),
            "image.show result should have 'image' field: {result:?}"
        );
    }

    Ok(())
}

// ── Image Set Default ──

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn test_image_set_default() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // Scan first
    let (_s, _r) = ws
        .rpc_call_streaming(
            "image.scan",
            json!({ "token": token }),
            Duration::from_secs(60),
        )
        .await?;

    // List images to get a model and version
    let list_resp = ws.rpc_call("image.list", json!({ "token": token })).await?;

    if let Some(result) = list_resp.get("result")
        && let Some(images) = result.as_array()
        && let Some(first) = images.first()
    {
        let model = first.get("model").and_then(|v| v.as_str()).unwrap_or("");
        let version = first.get("version").and_then(|v| v.as_str()).unwrap_or("");

        if !model.is_empty() && !version.is_empty() {
            let response = ws
                .rpc_call(
                    "image.set_default",
                    json!({
                        "token": token,
                        "model": model,
                        "version": version,
                    }),
                )
                .await?;

            assert!(
                response.get("result").is_some(),
                "set_default should succeed: {response:?}"
            );
        }
    }

    Ok(())
}

// ── Image Import (VM) ──

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn test_image_import_nonexistent_file() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let (_statuses, response) = ws
        .rpc_call_streaming(
            "image.import",
            json!({
                "token": token,
                "model": "ubuntu_linux",
                "version": "99.99",
                "path": "/nonexistent/path/virtioa.qcow2",
            }),
            Duration::from_secs(30),
        )
        .await?;

    assert!(
        response.get("error").is_some(),
        "importing nonexistent file should fail: {response:?}"
    );

    Ok(())
}

// ── Image Pull (Container) ──

#[tokio::test]
#[ignore = "requires a Docker daemon and internet access to pull an image"]
async fn test_image_pull_container() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // Pull nokia srlinux (already available locally as ghcr.io/nokia/srlinux:latest)
    let (_statuses, response) = ws
        .rpc_call_streaming(
            "image.pull",
            json!({
                "token": token,
                "model": "nokia_srlinux",
                "repo": "ghcr.io/nokia/srlinux",
                "tag": "latest",
            }),
            Duration::from_secs(120),
        )
        .await?;

    assert!(
        response.get("result").is_some(),
        "image.pull should succeed: {response:?}"
    );

    Ok(())
}

// ── Non-Admin Access Denied ──

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn test_image_scan_admin_only() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let admin_token = ws.login_admin().await?;

    // Create non-admin user
    let _r = ws
        .rpc_call(
            "user.create",
            json!({
                "token": admin_token,
                "username": "imguser",
                "password": "ImgUser123!",
                "is_admin": false,
            }),
        )
        .await?;

    let user_token = ws.login("imguser", "ImgUser123!").await?;

    // image.scan is streaming, but non-admin should get access denied
    let (_s, response) = ws
        .rpc_call_streaming(
            "image.scan",
            json!({ "token": user_token }),
            Duration::from_secs(10),
        )
        .await?;

    assert!(
        response.get("error").is_some(),
        "non-admin should be denied image.scan"
    );

    Ok(())
}
