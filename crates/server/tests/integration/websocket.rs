use anyhow::Result;
use serde_json::json;

use crate::helpers::test_server::TestServer;
use crate::helpers::ws_client::TestWsClient;

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
