use anyhow::{Context, Result, bail};
use std::time::Duration;

use shared::data::{ClientConfig, RedeployResponse, StatusKind, StatusMessage};
use shared::error::RpcErrorCode;
use shared::util::{Emoji, term_msg_surround};

use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

use super::up::{
    resolve_environment_variables, resolve_startup_scripts, resolve_user_scripts,
    resolve_ztp_configs,
};

/// Redeploy a single node: destroy and recreate with fresh ZTP
pub async fn redeploy(
    lab_name: &str,
    lab_id: &str,
    node_name: &str,
    manifest_path: &str,
    server_url: &str,
    config: &ClientConfig,
) -> Result<()> {
    term_msg_surround(&format!(
        "Redeploy node '{node_name}' - {lab_name}-{lab_id}"
    ));

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

    // Load and process manifest (resolve ZTP configs)
    let mut manifest = topology::Manifest::load_file(manifest_path)?;
    resolve_ztp_configs(&mut manifest, manifest_path)?;
    resolve_startup_scripts(&mut manifest, manifest_path)?;
    resolve_user_scripts(&mut manifest, manifest_path)?;
    resolve_environment_variables(&mut manifest)?;

    let manifest_value =
        serde_json::to_value(&manifest).context("Failed to serialize manifest to JSON")?;

    // Connect to server with extended timeout (15 minutes for redeploy)
    let timeout = Duration::from_secs(900);
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        timeout,
        config.server_connection.clone(),
    );

    let mut rpc_client = ws_client
        .connect()
        .await
        .context("Failed to connect to sherpad server")?;

    println!();

    let redeploy_request = RpcRequest::new(
        "redeploy",
        serde_json::json!({
            "lab_id": lab_id,
            "node_name": node_name,
            "manifest": manifest_value,
            "token": token,
        }),
    );

    let redeploy_response = rpc_client
        .call_streaming(redeploy_request, |msg_text| {
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
        .context("Redeploy RPC call failed")?;

    rpc_client.close().await.ok();

    // Handle errors
    if let Some(error) = redeploy_response.error {
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

        bail!("Redeploy operation failed");
    }

    let result_value = redeploy_response
        .result
        .context("No result in redeploy response")?;
    let result: RedeployResponse =
        serde_json::from_value(result_value).context("Failed to parse redeploy response")?;

    println!(
        "\n{} Node '{}' redeployed successfully in {}s",
        Emoji::Success,
        result.node_name,
        result.total_time_secs
    );

    Ok(())
}
