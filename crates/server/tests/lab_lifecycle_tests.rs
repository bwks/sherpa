mod helpers;

use anyhow::Result;
use serde_json::json;
use std::time::Duration;

use helpers::test_server::TestServer;
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
        .rpc_call_streaming("image.scan", json!({ "token": token }))
        .await?;
    assert!(
        response.get("error").is_none(),
        "image.scan failed: {:?}",
        response
    );
    Ok(())
}

// ── Container Lab Lifecycle ──

#[tokio::test]
#[ignore]
async fn test_container_lab_up_and_inspect() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;
    bootstrap_images(&mut ws, &token).await?;

    let lab_name = format!("test-cont-{}", std::process::id());
    let manifest = container_manifest(&lab_name);

    // Up the lab
    let (statuses, response) = ws
        .rpc_call_streaming_with_timeout(
            "up",
            json!({
                "token": token,
                "lab_id": lab_name,
                "manifest": manifest,
            }),
            Duration::from_secs(180),
        )
        .await?;

    assert!(
        response.get("error").is_none(),
        "lab up should succeed: {:?}",
        response
    );
    assert!(!statuses.is_empty(), "should have status messages during up");

    // Inspect the lab
    let inspect_resp = ws
        .rpc_call(
            "inspect",
            json!({
                "token": token,
                "lab_id": lab_name,
            }),
        )
        .await?;

    assert!(
        inspect_resp.get("result").is_some(),
        "inspect should succeed: {:?}",
        inspect_resp
    );

    // Cleanup: destroy the lab
    let (_s, destroy_resp) = ws
        .rpc_call_streaming_with_timeout(
            "destroy",
            json!({
                "token": token,
                "lab_id": lab_name,
            }),
            Duration::from_secs(60),
        )
        .await?;

    assert!(
        destroy_resp.get("error").is_none(),
        "destroy should succeed: {:?}",
        destroy_resp
    );

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_container_lab_down_and_resume() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;
    bootstrap_images(&mut ws, &token).await?;

    let lab_name = format!("test-dr-{}", std::process::id());
    let manifest = container_manifest(&lab_name);

    // Up
    let (_s, _r) = ws
        .rpc_call_streaming_with_timeout(
            "up",
            json!({
                "token": token,
                "lab_id": lab_name,
                "manifest": manifest,
            }),
            Duration::from_secs(180),
        )
        .await?;

    // Down
    let down_resp = ws
        .rpc_call(
            "down",
            json!({
                "token": token,
                "lab_id": lab_name,
            }),
        )
        .await?;

    assert!(
        down_resp.get("error").is_none(),
        "down should succeed: {:?}",
        down_resp
    );

    // Resume
    let resume_resp = ws
        .rpc_call(
            "resume",
            json!({
                "token": token,
                "lab_id": lab_name,
            }),
        )
        .await?;

    assert!(
        resume_resp.get("error").is_none(),
        "resume should succeed: {:?}",
        resume_resp
    );

    // Cleanup
    let (_s, _r) = ws
        .rpc_call_streaming_with_timeout(
            "destroy",
            json!({
                "token": token,
                "lab_id": lab_name,
            }),
            Duration::from_secs(60),
        )
        .await?;

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_container_lab_destroy() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;
    bootstrap_images(&mut ws, &token).await?;

    let lab_name = format!("test-dest-{}", std::process::id());
    let manifest = container_manifest(&lab_name);

    // Up
    let (_s, _r) = ws
        .rpc_call_streaming_with_timeout(
            "up",
            json!({
                "token": token,
                "lab_id": lab_name,
                "manifest": manifest,
            }),
            Duration::from_secs(180),
        )
        .await?;

    // Destroy
    let (_s, destroy_resp) = ws
        .rpc_call_streaming_with_timeout(
            "destroy",
            json!({
                "token": token,
                "lab_id": lab_name,
            }),
            Duration::from_secs(60),
        )
        .await?;

    assert!(
        destroy_resp.get("error").is_none(),
        "destroy should succeed: {:?}",
        destroy_resp
    );

    // Verify lab is gone - inspect should fail
    let inspect_resp = ws
        .rpc_call(
            "inspect",
            json!({
                "token": token,
                "lab_id": lab_name,
            }),
        )
        .await?;

    assert!(
        inspect_resp.get("error").is_some(),
        "inspect after destroy should fail"
    );

    Ok(())
}

// ── VM Lab Lifecycle ──

#[tokio::test]
#[ignore]
async fn test_vm_lab_up_and_destroy() -> Result<()> {
    // Skip if KVM is not available
    if !std::path::Path::new("/dev/kvm").exists() {
        eprintln!("Skipping VM test: /dev/kvm not available");
        return Ok(());
    }

    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;
    bootstrap_images(&mut ws, &token).await?;

    let lab_name = format!("test-vm-{}", std::process::id());
    let manifest = vm_manifest(&lab_name);

    // Up the VM lab
    let (statuses, response) = ws
        .rpc_call_streaming_with_timeout(
            "up",
            json!({
                "token": token,
                "lab_id": lab_name,
                "manifest": manifest,
            }),
            Duration::from_secs(300),
        )
        .await?;

    assert!(
        response.get("error").is_none(),
        "VM lab up should succeed: {:?}",
        response
    );
    assert!(!statuses.is_empty());

    // Cleanup
    let (_s, _r) = ws
        .rpc_call_streaming_with_timeout(
            "destroy",
            json!({
                "token": token,
                "lab_id": lab_name,
            }),
            Duration::from_secs(120),
        )
        .await?;

    Ok(())
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
        .rpc_call_streaming_with_timeout(
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
        .rpc_call_streaming_with_timeout(
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
        .rpc_call_streaming_with_timeout(
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
