use std::time::Duration;

use anyhow::{Context, bail};

use shared::data::{ClientConfig, DestroyResponse};
use shared::error::RpcErrorCode;
use shared::util::{Emoji, term_msg_surround};

use crate::cmd::destroy::display_destroy_results;
use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

/// Clean all resources for a lab (admin-only)
///
/// Unlike `destroy`, this command:
/// - Does not require the lab to exist in the database
/// - Skips ownership validation (admin-only)
/// - Tolerates missing resources
/// - Cleans everything matching the lab_id
pub async fn clean(lab_id: &str, server_url: &str, config: &ClientConfig) -> anyhow::Result<()> {
    term_msg_surround(&format!("Clean environment - {lab_id}"));

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

    println!(
        "\n{} Cleaning all resources for lab_id: {}\n",
        Emoji::Warning,
        lab_id
    );

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

    let clean_request = RpcRequest::new(
        "clean",
        serde_json::json!({
            "lab_id": lab_id,
            "token": token,
        }),
    );

    let clean_response = rpc_client
        .call(clean_request)
        .await
        .context("Clean RPC call failed")?;

    rpc_client.close().await.ok();

    // Handle errors
    if let Some(error) = clean_response.error {
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

        bail!("Clean operation failed");
    }

    let clean_result = clean_response
        .result
        .context("No result in clean response")?;
    let clean_data: DestroyResponse =
        serde_json::from_value(clean_result).context("Failed to parse clean response")?;

    display_destroy_results(&clean_data)?;

    Ok(())
}
