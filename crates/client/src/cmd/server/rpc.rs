use std::time::Duration;

use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::token::load_token;
use crate::ws_client::{RpcClient, RpcRequest, WebSocketClient};

use shared::data::ServerConnection;

/// Send `method` with `params` and the saved login token, and return its typed result.
///
/// # Errors
///
/// Fails when there is no saved token, `params` is not a JSON object, the server cannot be
/// reached, or it returns an error or a result that does not parse.
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
    let (mut rpc_client, request) =
        connect_with_request(method, params, server_url, server_connection).await?;
    let response = rpc_client.call(request).await;
    let _ = rpc_client.close().await;
    response?.into_result(&format!("{method} failed"))
}

/// As `rpc_call`, for a method that streams progress messages, each passed to `callback`
/// before the result arrives.
///
/// # Errors
///
/// As `rpc_call`.
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
    let (mut rpc_client, request) =
        connect_with_request(method, params, server_url, server_connection).await?;
    let response = rpc_client.call_streaming(request, callback).await;
    let _ = rpc_client.close().await;
    response?.into_result(&format!("{method} failed"))
}

/// Connect to the server, and build the request for `method` with `params` and the saved token.
async fn connect_with_request<P: Serialize>(
    method: &str,
    params: P,
    server_url: &str,
    server_connection: &ServerConnection,
) -> Result<(RpcClient, RpcRequest)> {
    let token = load_token()?;
    let mut params = serde_json::to_value(&params).context("Failed to serialize request params")?;
    params
        .as_object_mut()
        .with_context(|| format!("The parameters of {method} are not a JSON object"))?
        .insert("token".to_owned(), Value::String(token));

    let ws_client = WebSocketClient::new(
        server_url.to_owned(),
        Duration::from_secs(server_connection.timeout_secs),
        server_connection.clone(),
    );
    let rpc_client = ws_client.connect().await?;
    Ok((rpc_client, RpcRequest::new(method, params)))
}
