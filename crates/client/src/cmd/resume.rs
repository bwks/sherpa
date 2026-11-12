use anyhow::{Context, Result, bail};
use std::time::Duration;

use shared::data::{ClientConfig, LabNodeActionResponse};
use shared::konst::SHERPA_MANIFEST_FILE;
use shared::util::{Emoji, file_exists, term_msg_surround};
use topology::Manifest;

use super::progress::print_status;
use crate::cmd::download::download;
use crate::cmd::interrupt::{
    OperationTarget, StoppableOperation, Waited, call_streaming_or_interrupt,
};
use crate::cmd::outcome::{ensure_nodes_succeeded, print_link_reports};
use crate::cmd::tailscale_key::resolve_resume_tailscale_key;
use crate::table::render_tailnet_table;
use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

/// Start/poweron lab nodes via WebSocket RPC to sherpad server
#[tracing::instrument(skip_all, fields(%lab_name, %lab_id, ?node_name))]
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

    // `down` logged the Tailscale gateway out, so a whole-lab resume enrolls it again.
    let manifest = if file_exists(SHERPA_MANIFEST_FILE) {
        Some(Manifest::load_file(SHERPA_MANIFEST_FILE)?)
    } else {
        None
    };
    let tailscale_auth_key =
        resolve_resume_tailscale_key(node_name, manifest.as_ref(), server_url, config)?;

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
    if let Some(key) = tailscale_auth_key {
        params["tailscale_auth_key"] = serde_json::to_value(key)?;
    }

    let request = RpcRequest::new("resume", params);

    // Send request and wait for response
    let status_msg = match node_name {
        Some(name) => format!("Starting node {name}..."),
        None => "Starting lab nodes...".to_string(),
    };
    println!("{}", status_msg);
    // Resume streams readiness progress for cold-booted nodes before its response.
    let target = OperationTarget {
        operation: StoppableOperation::Resume,
        lab_id,
        token: &token,
        server_url,
        server_connection: &config.server_connection,
    };
    let response =
        match call_streaming_or_interrupt(&mut rpc_client, request, print_status, &target)
            .await
            .context("RPC call failed")?
        {
            Waited::Response(response) => response,
            Waited::Interrupted => bail!("Resume interrupted"),
        };

    // Close connection
    rpc_client.close().await.ok();

    // Handle response
    let action_response: LabNodeActionResponse = response.into_result("Resume operation failed")?;

    if let Some(status) = &action_response.tailscale {
        println!("{}", render_tailnet_table(status));
    }

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
        print_link_reports(&node_result.links);
    }

    // A booted node may offer a new host key, which the server recorded; refresh the lab
    // files so its known_hosts file matches.
    if action_response.results.iter().any(|result| result.success)
        && let Err(error) = download(lab_id, true, server_url, config).await
    {
        eprintln!(
            "{} Nodes started, but refreshing the lab files failed: {error:#}\n   Run: sherpa download {lab_id} --force",
            Emoji::Warning
        );
    }

    ensure_nodes_succeeded("started", &action_response.results)
}
