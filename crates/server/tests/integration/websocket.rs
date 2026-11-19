use anyhow::Result;
use serde_json::json;

use crate::helpers::test_server::TestServer;
use crate::helpers::ws_client::TestWsClient;

// ── Connection Tests ──

#[tokio::test]
async fn test_ws_multiple_connections() -> Result<()> {
    let server = TestServer::start().await?;

    let ws1 = TestWsClient::connect(&server).await?;
    let ws2 = TestWsClient::connect(&server).await?;

    assert_ne!(
        ws1.connection_id, ws2.connection_id,
        "Each connection should get a unique ID"
    );

    Ok(())
}

// ── RPC Dispatch Tests ──

#[tokio::test]
async fn test_rpc_unknown_method() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    let response = ws.rpc_call("nonexistent.method", json!({})).await?;

    let error = response.get("error").expect("should have error");
    // MethodNotFound = -32601
    assert_eq!(
        error.get("code").and_then(serde_json::Value::as_i64),
        Some(-32601)
    );
    assert!(
        error
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains("not found")
    );

    Ok(())
}

// ── Auth Method Tests ──

#[tokio::test]
async fn test_rpc_auth_validate_returns_user_info() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    let token = ws.login_admin().await?;

    let response = ws
        .rpc_call("auth.validate", json!({ "token": token }))
        .await?;

    let result = response.get("result").expect("should have result");
    assert_eq!(
        result.get("valid").and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        result.get("username").and_then(|v| v.as_str()),
        Some("admin")
    );
    assert_eq!(
        result.get("is_admin").and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert!(result.get("expires_at").is_some());

    Ok(())
}
