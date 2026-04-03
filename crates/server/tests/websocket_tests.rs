mod helpers;

use anyhow::Result;
use serde_json::json;

use helpers::test_server::TestServer;
use helpers::ws_client::TestWsClient;

// ── Connection Tests ──

#[tokio::test]
#[ignore]
async fn test_ws_connected_message() -> Result<()> {
    let server = TestServer::start().await?;
    let ws = TestWsClient::connect(&server).await?;

    // Connection ID should be a valid UUID
    assert!(!ws.connection_id.is_empty());
    assert!(
        uuid::Uuid::parse_str(&ws.connection_id).is_ok(),
        "connection_id should be a valid UUID"
    );

    Ok(())
}

#[tokio::test]
#[ignore]
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
#[ignore]
async fn test_rpc_unknown_method() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    let response = ws.rpc_call("nonexistent.method", json!({})).await?;

    let error = response.get("error").expect("should have error");
    // MethodNotFound = -32601
    assert_eq!(error.get("code").and_then(|v| v.as_i64()), Some(-32601));
    assert!(
        error
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains("not found")
    );

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_rpc_response_id_matches_request() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    // Make multiple requests and verify IDs match
    let r1 = ws
        .rpc_call(
            "auth.login",
            json!({"username": "admin", "password": "wrong"}),
        )
        .await?;
    let r2 = ws
        .rpc_call(
            "auth.login",
            json!({"username": "admin", "password": "wrong"}),
        )
        .await?;

    let id1 = r1.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let id2 = r2.get("id").and_then(|v| v.as_str()).unwrap_or("");

    assert_ne!(id1, id2, "Each response should have a unique ID");
    assert!(
        id1.starts_with("test-"),
        "Response ID should match request format"
    );
    assert!(
        id2.starts_with("test-"),
        "Response ID should match request format"
    );

    Ok(())
}

// ── Auth Method Tests ──

#[tokio::test]
#[ignore]
async fn test_rpc_auth_login_returns_jwt() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    let token = ws.login_admin().await?;

    // JWT tokens have 3 base64 parts separated by dots
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 3, "JWT should have 3 parts");

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_rpc_auth_validate_returns_user_info() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    let token = ws.login_admin().await?;

    let response = ws
        .rpc_call("auth.validate", json!({ "token": token }))
        .await?;

    let result = response.get("result").expect("should have result");
    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        result.get("username").and_then(|v| v.as_str()),
        Some("admin")
    );
    assert_eq!(result.get("is_admin").and_then(|v| v.as_bool()), Some(true));
    assert!(result.get("expires_at").is_some());

    Ok(())
}
