use anyhow::{Context, Result};
use std::time::Duration;

use shared::data::{ClientConfig, InspectResponse};
use shared::util::{Emoji, term_msg_surround, term_msg_underline};

use crate::table::{
    render_bridges_table, render_devices_table, render_lab_info_table, render_links_table,
    render_router_table, render_tailnet_table,
};
use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

/// Inspect lab via WebSocket RPC to sherpad server
#[tracing::instrument(skip_all, fields(%lab_name, %lab_id))]
pub async fn inspect(
    lab_name: &str,
    lab_id: &str,
    server_url: &str,
    config: &ClientConfig,
) -> Result<()> {
    term_msg_surround(&format!("Sherpa Environment - {lab_name}-{lab_id}"));

    println!("Connecting to server: {server_url}");
    println!("Requesting lab inspection from server...");
    let inspect_data = fetch_inspect(lab_id, server_url, config).await?;

    // Display results (similar format to original inspect command)
    if let Some(status) = &inspect_data.tailscale {
        println!("{}", render_tailnet_table(status));
    }

    let lab_info_table = render_lab_info_table(&inspect_data.lab_info);
    println!("{}", lab_info_table);
    if let Some(router) = &inspect_data.router {
        println!("{}", render_router_table(router));
    }
    // A lab being created or destroyed, or left so, says what to do with it.
    if let Err(not_ready) = inspect_data.lab_phase.ensure_ready() {
        println!("{} Not ready: {not_ready}", Emoji::Warning);
    }

    // Display active devices in table format
    if !inspect_data.devices.is_empty() {
        println!();
        let table = render_devices_table(&inspect_data.devices, &inspect_data.lab_info.id);
        println!("{table}");
    }

    // Display links
    if !inspect_data.links.is_empty() {
        println!();
        let table = render_links_table(&inspect_data.links);
        println!("{table}");
    }

    // Display bridges
    if !inspect_data.bridges.is_empty() {
        println!();
        let table = render_bridges_table(&inspect_data.bridges);
        println!("{table}");
    }

    // Display inactive devices
    if !inspect_data.inactive_devices.is_empty() {
        term_msg_underline("Inactive Nodes");
        for device in &inspect_data.inactive_devices {
            println!("{device}");
        }
    }

    Ok(())
}

/// The server's view of lab `lab_id`: its details, nodes and links, through the `inspect` RPC.
///
/// # Errors
///
/// Fails when there is no login token, the server cannot be reached, or it refuses the call.
#[tracing::instrument(level = "debug", skip_all, fields(%lab_id))]
pub async fn fetch_inspect(
    lab_id: &str,
    server_url: &str,
    config: &ClientConfig,
) -> Result<InspectResponse> {
    let token = load_token()?;
    let timeout = Duration::from_secs(config.server_connection.timeout_secs);
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        timeout,
        config.server_connection.clone(),
    );
    let mut rpc_client = ws_client
        .connect()
        .await
        .context("Failed to connect to sherpad server")?;
    let request = RpcRequest::new(
        "inspect",
        serde_json::json!({
            "lab_id": lab_id,
            "token": token,
        }),
    );
    let response = rpc_client.call(request).await.context("RPC call failed")?;
    rpc_client.close().await.ok();
    response.into_result("Inspection failed")
}
