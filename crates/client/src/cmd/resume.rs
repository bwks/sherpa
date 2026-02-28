use anyhow::{Context, Result, bail};
use std::time::Duration;

use shared::data::{ClientConfig, LabVmActionResponse};
use shared::error::RpcErrorCode;
use shared::util::{Emoji, term_msg_surround};

use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

/// Resume lab VMs via WebSocket RPC to sherpad server
pub async fn resume(
    lab_name: &str,
    lab_id: &str,
    server_url: &str,
    config: &ClientConfig,
) -> Result<()> {
    term_msg_surround(&format!("Resuming Environment - {lab_name}-{lab_id}"));

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
    let request = RpcRequest::new(
        "resume",
        serde_json::json!({
            "lab_id": lab_id,
            "token": token,
        }),
    );

    // Send request and wait for response
    println!("Resuming lab VMs...");
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

    let action_response: LabVmActionResponse =
        serde_json::from_value(result).context("Failed to parse resume response")?;

    for vm_result in &action_response.results {
        if vm_result.success {
            println!(
                "{} {}: {}",
                Emoji::Success,
                vm_result.name,
                vm_result.message
            );
        } else {
            eprintln!("{} {}: {}", Emoji::Error, vm_result.name, vm_result.message);
        }
    }

    Ok(())
}
