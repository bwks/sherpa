use anyhow::{Context, Result};
use std::io::{self, Write};
use std::time::Duration;

use shared::data::{ClientConfig, LabNodeActionResponse};
use shared::util::{Emoji, term_msg_surround};

use crate::cmd::outcome::ensure_nodes_succeeded;
use crate::table::render_tailnet_table;
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

    write_down_report(
        &action_response,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    )?;

    ensure_nodes_succeeded("stopped", &action_response.results)
}

/// Write down's Tailscale table and each node's outcome, failures to `err`.
fn write_down_report(
    response: &LabNodeActionResponse,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<()> {
    if let Some(status) = &response.tailscale {
        writeln!(out, "{}", render_tailnet_table(status))?;
    }
    for node_result in &response.results {
        if node_result.success {
            writeln!(
                out,
                "{} {}: {}",
                Emoji::Success,
                node_result.name,
                node_result.message
            )?;
        } else {
            writeln!(
                err,
                "{} {}: {}",
                Emoji::Error,
                node_result.name,
                node_result.message
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::{NodeActionResult, RouteApproval, TailnetState, TailnetStatus};

    fn node_result(name: &str, success: bool) -> NodeActionResult {
        NodeActionResult {
            name: name.to_owned(),
            success,
            message: if success { "stopped" } else { "not stopped" }.to_owned(),
            links: Vec::new(),
        }
    }

    fn report(response: &LabNodeActionResponse) -> (String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        write_down_report(response, &mut out, &mut err).unwrap();
        (
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn tailnet_status_013_down_prints_the_tailscale_table_the_server_sends() {
        let status = TailnetStatus {
            state: TailnetState::Stopped,
            routes: vec!["172.31.3.0/24".into()],
            approval: RouteApproval::Unknown,
            tailscale_ips: Vec::new(),
            warnings: Vec::new(),
        };
        let with_gateway = LabNodeActionResponse {
            tailscale: Some(status.clone()),
            results: vec![node_result("r1", true), node_result("tailscale", false)],
        };
        let (out, err) = report(&with_gateway);
        assert_eq!(
            out,
            format!(
                "{}\n{} r1: stopped\n",
                render_tailnet_table(&status),
                Emoji::Success
            )
        );
        assert_eq!(err, format!("{} tailscale: not stopped\n", Emoji::Error));

        let without_gateway = LabNodeActionResponse {
            tailscale: None,
            results: vec![node_result("r1", true)],
        };
        let (out, err) = report(&without_gateway);
        assert_eq!(out, format!("{} r1: stopped\n", Emoji::Success));
        assert_eq!(err, "");
    }
}
