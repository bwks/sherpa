use anyhow::{Context, Result};
use std::time::Duration;

use shared::data::{ClientConfig, LabNodeActionResponse};
use shared::util::{Emoji, term_msg_surround};

use crate::cmd::outcome::ensure_nodes_succeeded;
use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

/// Shutdown lab nodes via WebSocket RPC to sherpad server
#[tracing::instrument(skip_all, fields(%lab_name, %lab_id, ?node_name))]
pub async fn down(
    lab_name: &str,
    lab_id: &str,
    node_name: Option<&str>,
    server_url: &str,
    config: &ClientConfig,
) -> Result<()> {
    let action_msg = match node_name {
        Some(name) => format!("Stopping node {name} - {lab_name}-{lab_id}"),
        None => format!("Stopping environment - {lab_name}-{lab_id}"),
    };
    term_msg_surround(&action_msg);

    let token = load_token()?;

    // Create WebSocket client
    let timeout = Duration::from_secs(config.server_connection.timeout_secs);
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        timeout,
        config.server_connection.clone(),
    );

    // Connect to server
    println!("Connecting to server: {}", server_url);
    let mut rpc_client = ws_client
        .connect()
        .await
        .context("Failed to connect to sherpad server")?;

    // Create RPC request
    let mut params = serde_json::json!({
        "lab_id": lab_id,
        "token": token,
    });
    if let Some(name) = node_name {
        params["node_name"] = serde_json::Value::String(name.to_string());
    }

    let request = RpcRequest::new("down", params);

    // Send request and wait for response
    let status_msg = match node_name {
        Some(name) => format!("Stopping node {name}..."),
        None => "Stopping lab nodes...".to_string(),
    };
    println!("{}", status_msg);
    let response = rpc_client.call(request).await.context("RPC call failed")?;

    // Close connection
    rpc_client.close().await.ok();

    // Handle response
    let action_response: LabNodeActionResponse = response.into_result("Down operation failed")?;

    for node_result in &action_response.results {
        if node_result.success {
            println!(
                "{} {}: {}",
                Emoji::Success,
                node_result.name,
                node_result.message
            );
        } else {
            eprintln!(
                "{} {}: {}",
                Emoji::Error,
                node_result.name,
                node_result.message
            );
        }
    }

    ensure_nodes_succeeded("stopped", &action_response.results)
}
