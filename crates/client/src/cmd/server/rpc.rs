use std::time::Duration;

use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

use shared::data::ServerConnection;

/// Convenience helper: load token, connect via WebSocket, send an RPC call, close, return typed result.
pub async fn rpc_call<P, R>(
    method: &str,
    params: P,
    server_url: &str,
    server_connection: &ServerConnection,
) -> Result<R>
where
    P: Serialize,
    R: DeserializeOwned,
{
    let token = load_token().context("Not authenticated. Please login first.")?;

    // Inject token into params
    let mut params_value =
        serde_json::to_value(&params).context("Failed to serialize request params")?;

    if let Some(obj) = params_value.as_object_mut() {
        obj.insert("token".to_string(), serde_json::Value::String(token));
    }

    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        Duration::from_secs(30),
        server_connection.clone(),
    );

    let mut rpc_client = ws_client.connect().await?;

    let request = RpcRequest::new(method, params_value);
    let response = rpc_client.call(request).await?;

    let _ = rpc_client.close().await;

    // Check for RPC error
    if let Some(error) = response.error {
        if let Some(ctx) = &error.context {
            anyhow::bail!(
                "RPC error: {} (code: {})\n  Context: {}",
                error.message,
                error.code,
                ctx
            );
        } else {
            anyhow::bail!("RPC error: {} (code: {})", error.message, error.code);
        }
    }

    let result = response.result.context("No result in response")?;
    let typed_result: R =
        serde_json::from_value(result).context("Failed to deserialize response")?;

    Ok(typed_result)
}

/// Streaming RPC helper: load token, connect via WebSocket, send a streaming RPC call with
/// progress callback, close, return typed result.
pub async fn rpc_call_streaming<P, R, F>(
    method: &str,
    params: P,
    server_url: &str,
    server_connection: &ServerConnection,
    callback: F,
) -> Result<R>
where
    P: Serialize,
    R: DeserializeOwned,
    F: FnMut(&str),
{
    let token = load_token().context("Not authenticated. Please login first.")?;

    // Inject token into params
    let mut params_value =
        serde_json::to_value(&params).context("Failed to serialize request params")?;

    if let Some(obj) = params_value.as_object_mut() {
        obj.insert("token".to_string(), serde_json::Value::String(token));
    }

    // Extended timeout for downloads (15 minutes)
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        Duration::from_secs(900),
        server_connection.clone(),
    );

    let mut rpc_client = ws_client.connect().await?;

    let request = RpcRequest::new(method, params_value);
    let response = rpc_client.call_streaming(request, callback).await?;

    let _ = rpc_client.close().await;

    // Check for RPC error
    if let Some(error) = response.error {
        if let Some(ctx) = &error.context {
            anyhow::bail!(
                "RPC error: {} (code: {})\n  Context: {}",
                error.message,
                error.code,
                ctx
            );
        } else {
            anyhow::bail!("RPC error: {} (code: {})", error.message, error.code);
        }
    }

    let result = response.result.context("No result in response")?;
    let typed_result: R =
        serde_json::from_value(result).context("Failed to deserialize response")?;

    Ok(typed_result)
}
