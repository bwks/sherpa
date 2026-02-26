use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::fs;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use shared::data::{Config, StatusKind, StatusMessage, UpResponse};
use shared::error::RpcErrorCode;
use shared::konst::{SHERPA_SSH_CONFIG_FILE, SHERPA_SSH_PRIVATE_KEY_FILE};
use shared::util::{
    Emoji, get_cwd, get_username, render_lab_info_table, render_nodes_table, term_msg_surround,
    term_msg_underline,
};

use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

/// Start lab  to sherpad server with streaming progress updates
///
/// Flow:
/// 1. Load manifest from TOML file
/// 2. Convert manifest to JSON
/// 3. Connect to server with extended timeout (15 minutes for long-running operation)
/// 4. Send RPC request and stream progress updates
/// 5. Display final results
pub async fn up(
    lab_name: &str,
    lab_id: &str,
    manifest_path: &str,
    server_url: &str,
    config: &Config,
) -> Result<()> {
    term_msg_surround(&format!("Start Lab - {lab_name}-{lab_id}"));

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

    // Load and parse manifest
    println!("\nLoading manifest from: {}\n", manifest_path);

    let manifest_str = fs::read_to_string(manifest_path)
        .with_context(|| format!("Failed to read manifest file: {}", manifest_path))?;

    // Parse TOML to JSON Value
    let manifest_value: serde_json::Value = toml::from_str(&manifest_str)
        .with_context(|| format!("Failed to parse manifest TOML: {}", manifest_path))?;

    // Extended timeout for long-running up operation (15 minutes)
    let timeout = Duration::from_secs(900);
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        timeout,
        config.server_connection.clone(),
    );

    // Connect
    let mut rpc_client = ws_client
        .connect()
        .await
        .context("Failed to connect to sherpad server")?;

    // Get current username (for display only - server uses token)
    let _username = get_username()?;

    // Create RPC request with authentication token
    let up_request = RpcRequest::new(
        "up",
        serde_json::json!({
            "lab_id": lab_id,
            "manifest": manifest_value,
            "token": token,
        }),
    );

    println!("Starting lab creation...\n");

    // Call streaming RPC with progress callback
    let up_response = rpc_client
        .call_streaming(up_request, |msg_text| {
            // Parse and display progress messages
            if let Ok(status_msg) = serde_json::from_str::<StatusMessage>(msg_text) {
                if status_msg.r#type == "status" {
                    // Display phase header if available
                    if let Some(phase) = &status_msg.phase {
                        println!(); // Blank line before phase header
                        term_msg_underline(phase);
                    }
                    // Display the message with kind-appropriate emoji
                    let emoji = match status_msg.kind {
                        StatusKind::Progress => Emoji::Progress,
                        StatusKind::Done => Emoji::Success,
                        StatusKind::Info => Emoji::Info,
                        StatusKind::Waiting => Emoji::Hourglass,
                    };
                    println!("{} {}", emoji, status_msg.message);
                }
            } else if let Ok(log_msg) = serde_json::from_str::<LogMessage>(msg_text)
                && log_msg.r#type == "log"
            {
                // Display log messages (for debugging)
                println!("[LOG] {}", log_msg.message);
            }
        })
        .await
        .context("Up RPC call failed")?;

    // Close connection
    rpc_client.close().await.ok();

    // Handle errors
    if let Some(error) = up_response.error {
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

        bail!("Lab creation failed");
    }

    let up_result = up_response.result.context("No result in up response")?;
    let up_data: UpResponse =
        serde_json::from_value(up_result).context("Failed to parse up response")?;

    // Write SSH config to local directory
    match get_cwd() {
        Ok(cwd) => {
            let local_ssh_config_path = format!("{}/{}", cwd, SHERPA_SSH_CONFIG_FILE);
            match fs::write(&local_ssh_config_path, &up_data.ssh_config) {
                Ok(_) => {
                    println!(
                        "\n{} SSH config created: {}",
                        Emoji::Success,
                        local_ssh_config_path
                    );
                }
                Err(e) => {
                    println!(
                        "\n{} Warning: Failed to create local SSH config: {}",
                        Emoji::Warning,
                        e
                    );
                }
            }
        }
        Err(e) => {
            println!(
                "\n{} Warning: Could not determine working directory: {}",
                Emoji::Warning,
                e
            );
        }
    }

    // Write SSH private key to local directory with 0600 permissions
    match get_cwd() {
        Ok(cwd) => {
            let local_ssh_key_path = format!("{}/{}", cwd, SHERPA_SSH_PRIVATE_KEY_FILE);
            match fs::write(&local_ssh_key_path, &up_data.ssh_private_key) {
                Ok(_) => {
                    // Set Unix permissions to 0600 (owner read/write only)
                    #[cfg(unix)]
                    {
                        if let Err(e) = fs::set_permissions(
                            &local_ssh_key_path,
                            fs::Permissions::from_mode(0o600),
                        ) {
                            println!(
                                "\n{} Warning: Failed to set permissions on SSH key: {}",
                                Emoji::Warning,
                                e
                            );
                        }
                    }
                    println!(
                        "{} SSH private key created: {}",
                        Emoji::Success,
                        local_ssh_key_path
                    );
                }
                Err(e) => {
                    println!(
                        "\n{} Warning: Failed to create local SSH private key: {}",
                        Emoji::Warning,
                        e
                    );
                }
            }
        }
        Err(e) => {
            println!(
                "\n{} Warning: Could not determine working directory: {}",
                Emoji::Warning,
                e
            );
        }
    }

    // Display results
    display_up_results(&up_data)?;

    Ok(())
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

    // Lab Info
    let lab_info_table = render_lab_info_table(&response.lab_info);
    println!("{}", lab_info_table);

    // Node information
    if !response.nodes.is_empty() {
        println!("\n{}", render_nodes_table(&response.nodes));
    }

    // Timing
    println!("\nDuration: {:.2}s", response.total_time_secs);

    // Errors (if any)
    if !response.errors.is_empty() {
        println!("\n{} Warnings/Errors:", Emoji::Warning);
        for error in &response.errors {
            let icon = if error.is_critical {
                Emoji::Error
            } else {
                Emoji::Warning
            };
            println!("  {} [{}] {}", icon, error.phase, error.message);
        }
    }

    // Final status
    if response.success {
        println!("\n{} Lab created successfully!\n", Emoji::Success);
    } else {
        println!(
            "\n{} Lab partially created - review errors above\n",
            Emoji::Warning
        );
    }

    Ok(())
}
