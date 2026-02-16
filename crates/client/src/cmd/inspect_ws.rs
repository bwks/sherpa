use anyhow::{Context, Result, bail};
use std::time::Duration;

use shared::data::{Config, InspectResponse};
use shared::error::RpcErrorCode;
use shared::util::{
    Emoji, get_username, render_devices_table, render_lab_info_table, term_msg_surround,
    term_msg_underline,
};

use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

/// Inspect lab via WebSocket RPC to sherpad server
pub async fn inspect_ws(
    lab_name: &str,
    lab_id: &str,
    server_url: &str,
    config: &Config,
) -> Result<()> {
    term_msg_surround(&format!("Sherpa Environment - {lab_name}-{lab_id}"));

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
    let ws_client = WebSocketClient::new(server_url.to_string(), timeout);

    // Connect to server
    println!("Connecting to server: {}", server_url);
    let mut rpc_client = ws_client
        .connect()
        .await
        .context("Failed to connect to sherpad server")?;

    // Get current username (for display purposes only - server uses token)
    let _username = get_username()?;

    // Create RPC request with authentication token
    let request = RpcRequest::new(
        "inspect",
        serde_json::json!({
            "lab_id": lab_id,
            "token": token,
        }),
    );

    // Send request and wait for response
    println!("Requesting lab inspection from server...");
    let response = rpc_client.call(request).await.context("RPC call failed")?;

    // Close connection
    rpc_client.close().await.ok(); // Ignore close errors

    // Handle response
    if let Some(error) = response.error {
        // Pretty-print error with context
        eprintln!("\n{} Server Error:", Emoji::Error);
        eprintln!("   Message: {}", error.message);
        eprintln!("   Code: {}", error.code);
        if let Some(context) = error.context {
            eprintln!("   Context:\n{}", context);
        }

        // Check for authentication errors
        if error.code == RpcErrorCode::AuthRequired {
            eprintln!(
                "\n{} Your authentication token has expired or is invalid",
                Emoji::Error
            );
            eprintln!("   Please run: sherpa login");
        }

        bail!("Inspection failed");
    }

    let result = response.result.context("No result in response")?;

    // Deserialize InspectResponse
    let inspect_data: InspectResponse =
        serde_json::from_value(result).context("Failed to parse inspect response")?;

    // Display results (similar format to original inspect command)
    let lab_info_table = render_lab_info_table(&inspect_data.lab_info);
    println!("{}", lab_info_table);

    // Display active devices in table format
    if !inspect_data.devices.is_empty() {
        println!();
        let table = render_devices_table(&inspect_data.devices);
        println!("{}", table);
    }

    // Display inactive devices
    if !inspect_data.inactive_devices.is_empty() {
        term_msg_underline("Inactive Devices");
        for device in &inspect_data.inactive_devices {
            println!("{}", device);
        }
    }

    Ok(())
}
