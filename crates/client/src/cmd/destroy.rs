use anyhow::{Context, Result, bail};
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use shared::data::{
    ClientConfig, DestroyResponse, InspectResponse, NodeInfo, StatusKind, StatusMessage,
};
use shared::error::RpcErrorCode;
use shared::konst::{SHERPA_SSH_CONFIG_FILE, SHERPA_SSH_PRIVATE_KEY_FILE};
use shared::util::{
    Emoji, display_destroy_results, file_exists, get_cwd, get_username, remove_lab_ssh_include,
    render_lab_info_table, render_nodes_table, term_msg_surround,
};

use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

/// Destroy lab  to sherpad server
///
/// Flow:
/// 1. Inspect lab to show what will be destroyed
/// 2. Ask for user confirmation
/// 3. If confirmed, destroy lab and show detailed results
pub async fn destroy(
    lab_name: &str,
    lab_id: &str,
    server_url: &str,
    config: &ClientConfig,
    yes: bool,
) -> Result<()> {
    term_msg_surround(&format!("Destroy environment - {lab_name}-{lab_id}"));

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

    // Phase 1: Inspect lab to show what will be destroyed
    println!("\nFetching lab details...\n");

    let timeout = Duration::from_secs(config.server_connection.timeout_secs);
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        timeout,
        config.server_connection.clone(),
    );

    // Connect and inspect
    let mut rpc_client = ws_client
        .connect()
        .await
        .context("Failed to connect to sherpad server")?;

    // Get current username (for display only - server uses token)
    let _username = get_username()?;

    let inspect_request = RpcRequest::new(
        "inspect",
        serde_json::json!({
            "lab_id": lab_id,
            "token": token.clone(),
        }),
    );

    let inspect_response = rpc_client
        .call(inspect_request)
        .await
        .context("Inspect RPC call failed")?;

    // Handle inspect errors
    if let Some(error) = inspect_response.error {
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

        bail!("Failed to inspect lab before destroy");
    }

    let inspect_result = inspect_response
        .result
        .context("No result in inspect response")?;
    let inspect_data: InspectResponse =
        serde_json::from_value(inspect_result).context("Failed to parse inspect response")?;

    // Display lab details
    let lab_info_table = render_lab_info_table(&inspect_data.lab_info);
    println!("{}", lab_info_table);

    // Convert DeviceInfo to NodeInfo for table rendering
    let nodes: Vec<NodeInfo> = inspect_data
        .devices
        .iter()
        .map(|device| NodeInfo {
            name: device.name.clone(),
            kind: device.kind.to_string(),
            model: device.model,
            status: device.state,
            ip_address: if device.mgmt_ipv4.is_empty() {
                None
            } else {
                Some(device.mgmt_ipv4.clone())
            },
            ssh_port: Some(22), // Default SSH port
        })
        .collect();

    if !nodes.is_empty() {
        println!("\n{}", render_nodes_table(&nodes));
    }

    let device_count = inspect_data.devices.len();

    // Phase 2: Ask for confirmation (skip if --yes flag is set)
    if !yes && !confirm_destroy(lab_name, lab_id, device_count)? {
        println!("\n{} Destroy operation cancelled by user", Emoji::Warning);
        return Ok(());
    }

    // Phase 3: Destroy lab
    println!("\n{} Destroying lab resources...\n", Emoji::Warning);

    let destroy_request = RpcRequest::new(
        "destroy",
        serde_json::json!({
            "lab_id": lab_id,
            "token": token,
        }),
    );

    let destroy_response = rpc_client
        .call_streaming(destroy_request, |msg_text| {
            if let Ok(status_msg) = serde_json::from_str::<StatusMessage>(msg_text)
                && status_msg.r#type == "status"
            {
                let emoji = match status_msg.kind {
                    StatusKind::Progress => Emoji::Progress,
                    StatusKind::Done => Emoji::Success,
                    StatusKind::Info => Emoji::Info,
                    StatusKind::Waiting => Emoji::Hourglass,
                };
                println!("{} {}", emoji, status_msg.message);
            }
        })
        .await
        .context("Destroy RPC call failed")?;

    // Close connection
    rpc_client.close().await.ok();

    // Handle destroy errors
    if let Some(error) = destroy_response.error {
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

        bail!("Destroy operation failed");
    }

    let destroy_result = destroy_response
        .result
        .context("No result in destroy response")?;
    let destroy_data: DestroyResponse =
        serde_json::from_value(destroy_result).context("Failed to parse destroy response")?;

    // Display detailed results
    display_destroy_results(&destroy_data)?;

    // Clean up local SSH config
    match get_cwd() {
        Ok(cwd) => {
            let local_ssh_config_path = Path::new(&cwd)
                .join(SHERPA_SSH_CONFIG_FILE)
                .to_string_lossy()
                .to_string();
            if file_exists(&local_ssh_config_path) {
                match fs::remove_file(&local_ssh_config_path) {
                    Ok(_) => {
                        println!(
                            "\n{} Local SSH config deleted: {}",
                            Emoji::Success,
                            local_ssh_config_path
                        );
                    }
                    Err(e) => {
                        println!(
                            "\n{} Warning: Failed to delete local SSH config: {}",
                            Emoji::Warning,
                            e
                        );
                    }
                }
            }
            // Silent success if file doesn't exist - idempotent
        }
        Err(e) => {
            println!(
                "\n{} Warning: Could not determine working directory: {}",
                Emoji::Warning,
                e
            );
        }
    }

    // Clean up local SSH private key
    match get_cwd() {
        Ok(cwd) => {
            let local_ssh_key_path = Path::new(&cwd)
                .join(SHERPA_SSH_PRIVATE_KEY_FILE)
                .to_string_lossy()
                .to_string();
            if file_exists(&local_ssh_key_path) {
                match fs::remove_file(&local_ssh_key_path) {
                    Ok(_) => {
                        println!(
                            "{} Local SSH private key deleted: {}",
                            Emoji::Success,
                            local_ssh_key_path
                        );
                    }
                    Err(e) => {
                        println!(
                            "\n{} Warning: Failed to delete local SSH private key: {}",
                            Emoji::Warning,
                            e
                        );
                    }
                }
            }
            // Silent success if file doesn't exist - idempotent
        }
        Err(e) => {
            println!(
                "\n{} Warning: Could not determine working directory: {}",
                Emoji::Warning,
                e
            );
        }
    }

    // Remove this lab's Include from ~/.ssh/sherpa_lab_hosts
    match get_cwd() {
        Ok(cwd) => {
            let local_ssh_config_path = Path::new(&cwd)
                .join(SHERPA_SSH_CONFIG_FILE)
                .to_string_lossy()
                .to_string();
            match remove_lab_ssh_include(&local_ssh_config_path) {
                Ok(_) => {
                    println!(
                        "{} SSH config removed from ~/.ssh/sherpa_lab_hosts",
                        Emoji::Success
                    );
                }
                Err(e) => {
                    println!(
                        "\n{} Warning: Failed to clean up ~/.ssh/sherpa_lab_hosts: {}",
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

    Ok(())
}

/// Ask user for confirmation before destroying lab
fn confirm_destroy(lab_name: &str, lab_id: &str, device_count: usize) -> Result<bool> {
    println!(
        "\n{} WARNING: This will permanently destroy all lab resources!",
        Emoji::Warning
    );
    print!(
        "\nAre you sure you want to destroy lab {}-{} ({} devices)? [y/N]: ",
        lab_name, lab_id, device_count
    );
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let input = input.trim().to_lowercase();
    Ok(input == "y" || input == "yes")
}
