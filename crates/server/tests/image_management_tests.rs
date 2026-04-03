mod helpers;

use anyhow::Result;
use serde_json::json;

use helpers::test_server::TestServer;
use helpers::ws_client::TestWsClient;

// ── Image Scan ──

#[tokio::test]
#[ignore]
async fn test_image_scan_discovers_images() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let (statuses, response) = ws
        .rpc_call_streaming("image.scan", json!({ "token": token }))
        .await?;

    assert!(
        response.get("result").is_some(),
        "image.scan should succeed: {:?}",
        response
    );

    // Should have found some status messages during scan
    assert!(!statuses.is_empty(), "scan should produce status messages");

    Ok(())
}

// ── Image List ──

#[tokio::test]
#[ignore]
async fn test_image_list_returns_images() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // First scan to register images
    let (_s, _r) = ws
        .rpc_call_streaming("image.scan", json!({ "token": token }))
        .await?;

    // List all images
    let response = ws
        .rpc_call("image.list", json!({ "token": token }))
        .await?;

    assert!(
        response.get("result").is_some(),
        "image.list should succeed: {:?}",
        response
    );

    Ok(())
}

// ── Image Show ──

#[tokio::test]
#[ignore]
async fn test_image_show_returns_config() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // Scan first
    let (_s, _r) = ws
        .rpc_call_streaming("image.scan", json!({ "token": token }))
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
    if response.get("result").is_some() {
        let result = response.get("result").unwrap();
        assert!(
            result.get("model").is_some() || result.get("name").is_some(),
            "image.show result should have model info"
        );
    }

    Ok(())
}

// ── Image Set Default ──

#[tokio::test]
#[ignore]
async fn test_image_set_default() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // Scan first
    let (_s, _r) = ws
        .rpc_call_streaming("image.scan", json!({ "token": token }))
        .await?;

    // List images to get a model and version
    let list_resp = ws
        .rpc_call("image.list", json!({ "token": token }))
        .await?;

    if let Some(result) = list_resp.get("result") {
        if let Some(images) = result.as_array() {
            if let Some(first) = images.first() {
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
                        "set_default should succeed: {:?}",
                        response
                    );
                }
            }
        }
    }

    Ok(())
}

// ── Image Import (VM) ──

#[tokio::test]
#[ignore]
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
        )
        .await?;

    assert!(
        response.get("error").is_some(),
        "importing nonexistent file should fail: {:?}",
        response
    );

    Ok(())
}

// ── Image Pull (Container) ──

#[tokio::test]
#[ignore]
async fn test_image_pull_container() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // Pull alpine which should already be available locally
    let (statuses, response) = ws
        .rpc_call_streaming(
            "image.pull",
            json!({
                "token": token,
                "model": "nokia_srlinux",
            }),
        )
        .await?;

    assert!(
        response.get("result").is_some(),
        "image.pull should succeed: {:?}",
        response
    );

    // Should have status messages showing pull progress
    assert!(!statuses.is_empty(), "pull should produce status messages");

    Ok(())
}

// ── Non-Admin Access Denied ──

#[tokio::test]
#[ignore]
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
        .rpc_call_streaming("image.scan", json!({ "token": user_token }))
        .await?;

    assert!(
        response.get("error").is_some(),
        "non-admin should be denied image.scan"
    );

    Ok(())
}
