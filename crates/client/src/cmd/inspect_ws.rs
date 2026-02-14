use anyhow::{Context, Result, bail};
use std::time::Duration;

use shared::data::{Config, InspectResponse};
use shared::konst::EMOJI_BAD;
use shared::util::{get_username, term_msg_surround, term_msg_underline};

use crate::ws_client::{RpcRequest, WebSocketClient};

/// Inspect lab via WebSocket RPC to sherpad server
pub async fn inspect_ws(
    lab_name: &str,
    lab_id: &str,
    server_url: &str,
    config: &Config,
) -> Result<()> {
    term_msg_surround(&format!(
        "Sherpa Environment - {lab_name}-{lab_id} (via WebSocket RPC)"
    ));

    // Create WebSocket client
    let timeout = Duration::from_secs(config.server_connection.timeout_secs);
    let ws_client = WebSocketClient::new(server_url.to_string(), timeout);

    // Connect to server
    println!("Connecting to server: {}", server_url);
    let mut rpc_client = ws_client
        .connect()
        .await
        .context("Failed to connect to sherpad server")?;

    // Get current username
    let username = get_username()?;

    // Create RPC request
    let request = RpcRequest::new(
        "inspect",
        serde_json::json!({
            "lab_id": lab_id,
            "username": username,
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
        eprintln!("\n{EMOJI_BAD} Server Error:");
        eprintln!("   Message: {}", error.message);
        eprintln!("   Code: {}", error.code);
        if let Some(context) = error.context {
            eprintln!("   Context:\n{}", context);
        }
        bail!("Inspection failed");
    }

    let result = response.result.context("No result in response")?;

    // Deserialize InspectResponse
    let inspect_data: InspectResponse =
        serde_json::from_value(result).context("Failed to parse inspect response")?;

    // Display results (similar format to original inspect command)
    term_msg_underline("Lab Info");
    println!("{}", inspect_data.lab_info);

    // Display active devices
    for device in &inspect_data.devices {
        term_msg_underline(&device.name);
        println!("Model: {}", device.model);
        println!("Kind: {}", device.kind);
        println!("Active: {}", device.active);
        if !device.mgmt_ip.is_empty() {
            println!("Mgmt IP: {}", device.mgmt_ip);
        }
        for disk in &device.disks {
            println!("Disk: {}", disk);
        }
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
