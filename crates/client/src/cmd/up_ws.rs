use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::fs;
use std::time::Duration;

use shared::data::{Config, UpResponse};
use shared::konst::{EMOJI_BAD, EMOJI_GOOD, EMOJI_WARN, SHERPA_SSH_CONFIG_FILE};
use shared::util::{get_cwd, term_msg_surround};

use crate::ws_client::{RpcRequest, WebSocketClient};

/// Start lab via WebSocket RPC to sherpad server with streaming progress updates
///
/// Flow:
/// 1. Load manifest from TOML file
/// 2. Convert manifest to JSON
/// 3. Connect to server with extended timeout (15 minutes for long-running operation)
/// 4. Send RPC request and stream progress updates
/// 5. Display final results
pub async fn up_ws(
    lab_name: &str,
    lab_id: &str,
    manifest_path: &str,
    server_url: &str,
    _config: &Config,
) -> Result<()> {
    term_msg_surround(&format!("Start Lab - {lab_name}-{lab_id} (via WebSocket RPC)"));

    // Load and parse manifest
    println!("\nLoading manifest from: {}\n", manifest_path);

    let manifest_str = fs::read_to_string(manifest_path)
        .with_context(|| format!("Failed to read manifest file: {}", manifest_path))?;

    // Parse TOML to JSON Value
    let manifest_value: serde_json::Value = toml::from_str(&manifest_str)
        .with_context(|| format!("Failed to parse manifest TOML: {}", manifest_path))?;

    // Extended timeout for long-running up operation (15 minutes)
    let timeout = Duration::from_secs(900);
    let ws_client = WebSocketClient::new(server_url.to_string(), timeout);

    // Connect
    let mut rpc_client = ws_client
        .connect()
        .await
        .context("Failed to connect to sherpad server")?;

    // Create RPC request
    let up_request = RpcRequest::new(
        "up",
        serde_json::json!({
            "lab_id": lab_id,
            "manifest": manifest_value,
        }),
    );

    println!("Starting lab creation...\n");

    // Call streaming RPC with progress callback
    let up_response = rpc_client
        .call_streaming(up_request, |msg_text| {
            // Parse and display progress messages
            if let Ok(status_msg) = serde_json::from_str::<StatusMessage>(msg_text) {
                if status_msg.r#type == "status" {
                    // Display phase progress if available
                    if let Some(phase) = &status_msg.phase {
                        println!("[{}] {}", phase, status_msg.message);
                    } else {
                        println!("{}", status_msg.message);
                    }
                }
            } else if let Ok(log_msg) = serde_json::from_str::<LogMessage>(msg_text) {
                if log_msg.r#type == "log" {
                    // Display log messages (for debugging)
                    println!("[LOG] {}", log_msg.message);
                }
            }
        })
        .await
        .context("Up RPC call failed")?;

    // Close connection
    rpc_client.close().await.ok();

    // Handle errors
    if let Some(error) = up_response.error {
        eprintln!("\n{EMOJI_BAD} Server Error:");
        eprintln!("   Message: {}", error.message);
        eprintln!("   Code: {}", error.code);
        if let Some(context) = error.context {
            eprintln!("   Context:\n{}", context);
        }
        bail!("Lab creation failed");
    }

    let up_result = up_response
        .result
        .context("No result in up response")?;
    let up_data: UpResponse =
        serde_json::from_value(up_result).context("Failed to parse up response")?;

    // Write SSH config to local directory
    match get_cwd() {
        Ok(cwd) => {
            let local_ssh_config_path = format!("{}/{}", cwd, SHERPA_SSH_CONFIG_FILE);
            match fs::write(&local_ssh_config_path, &up_data.ssh_config) {
                Ok(_) => {
                    println!("\n{} SSH config created: {}", EMOJI_GOOD, local_ssh_config_path);
                }
                Err(e) => {
                    println!(
                        "\n{} Warning: Failed to create local SSH config: {}",
                        EMOJI_WARN, e
                    );
                }
            }
        }
        Err(e) => {
            println!(
                "\n{} Warning: Could not determine working directory: {}",
                EMOJI_WARN, e
            );
        }
    }

    // Display results
    display_up_results(&up_data)?;

    Ok(())
}

/// Helper struct for deserializing status messages
#[derive(Deserialize)]
struct StatusMessage {
    r#type: String,
    message: String,
    phase: Option<String>,
}

/// Helper struct for deserializing log messages
#[derive(Deserialize)]
struct LogMessage {
    r#type: String,
    message: String,
}

/// Display lab creation results
fn display_up_results(response: &UpResponse) -> Result<()> {
    println!();
    term_msg_surround("Lab Creation Results");

    // Summary
    let summary = &response.summary;
    println!("\nResources Created:");
    println!("  Containers: {}", summary.containers_created);
    println!("  VMs: {}", summary.vms_created);
    println!("  Networks: {}", summary.networks_created);
    println!("  Bridges: {}", summary.bridges_created);
    println!("  Interfaces: {}", summary.interfaces_created);

    // Phases completed
    println!("\nPhases Completed: {}", response.phases_completed.len());

    // Node information
    if !response.nodes.is_empty() {
        println!("\nNodes:");
        for node in &response.nodes {
            let status_icon = match node.status.as_str() {
                "running" => EMOJI_GOOD,
                "created" => EMOJI_WARN,
                _ => EMOJI_BAD,
            };
            
            println!("  {} {} ({})", status_icon, node.name, node.kind);
            if let Some(ip) = &node.ip_address {
                println!("      IP: {}", ip);
            }
            if let Some(ssh_port) = node.ssh_port {
                println!("      SSH: localhost:{}", ssh_port);
            }
        }
    }

    // Timing
    println!("\nDuration: {:.2}s", response.total_time_secs);

    // Errors (if any)
    if !response.errors.is_empty() {
        println!("\n{EMOJI_WARN} Warnings/Errors:");
        for error in &response.errors {
            let icon = if error.is_critical { EMOJI_BAD } else { EMOJI_WARN };
            println!("  {} [{}] {}", icon, error.phase, error.message);
        }
    }

    // Final status
    if response.success {
        println!("\n{EMOJI_GOOD} Lab created successfully!\n");
    } else {
        println!("\n{EMOJI_WARN} Lab partially created - review errors above\n");
    }

    Ok(())
}
