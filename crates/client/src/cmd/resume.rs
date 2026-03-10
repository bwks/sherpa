use anyhow::{Context, Result, bail};
use std::time::Duration;

use shared::data::{ClientConfig, LabNodeActionResponse};
use shared::error::RpcErrorCode;
use shared::util::{Emoji, term_msg_surround};

use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

/// Start/poweron lab nodes via WebSocket RPC to sherpad server
pub async fn resume(
    lab_name: &str,
    lab_id: &str,
    node_name: Option<&str>,
    server_url: &str,
    config: &ClientConfig,
) -> Result<()> {
    let action_msg = match node_name {
        Some(name) => format!("Starting node {name} - {lab_name}-{lab_id}"),
        None => format!("Starting environment - {lab_name}-{lab_id}"),
    };
    term_msg_surround(&action_msg);

    // Load authentication token
    let token = match load_token() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("\n{} Authentication required", Emoji::Error);
            eprintln!("   Please run: sherpa login");
            eprintln!("   Error: {}", e);
            bail!("Authentication token not found");
        }
    };

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

    let request = RpcRequest::new("resume", params);

    // Send request and wait for response
    let status_msg = match node_name {
        Some(name) => format!("Starting node {}...", name),
        None => "Starting lab nodes...".to_string(),
    };
    println!("{}", status_msg);
    let response = rpc_client.call(request).await.context("RPC call failed")?;

    // Close connection
    rpc_client.close().await.ok();

    // Handle response
    if let Some(error) = response.error {
        eprintln!("\n{} Server Error:", Emoji::Error);
        eprintln!("   Message: {}", error.message);
        eprintln!("   Code: {}", error.code);
        if let Some(context) = error.context {
            eprintln!("   Context:\n{}", context);
        }

        if error.code == RpcErrorCode::AuthRequired {
            eprintln!(
                "\n{} Your authentication token has expired or is invalid",
                Emoji::Error
            );
            eprintln!("   Please run: sherpa login");
        }

        bail!("Resume operation failed");
    }

    let result = response.result.context("No result in response")?;

    let action_response: LabNodeActionResponse =
        serde_json::from_value(result).context("Failed to parse resume response")?;

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

    Ok(())
}
