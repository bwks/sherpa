use anyhow::{Context, Result};

use shared::data::{DestroyResponse, ServerConnection};
use shared::util::{Emoji, display_destroy_results, term_msg_surround};

use crate::common::rpc::RpcClient;
use crate::token::load_token;

/// Clean all resources for a lab (admin-only)
///
/// Unlike `destroy`, this command:
/// - Does not require the lab to exist in the database
/// - Skips ownership validation (admin-only)
/// - Tolerates missing resources
/// - Cleans everything matching the lab_id
pub async fn clean(
    lab_id: &str,
    server_url: &str,
    server_connection: &ServerConnection,
) -> Result<()> {
    term_msg_surround(&format!("Clean environment - {lab_id}"));

    let token = match load_token() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("\n{} Authentication required", Emoji::Error);
            eprintln!("   Please run: sherpa login");
            eprintln!("   Error: {}", e);
            anyhow::bail!("Authentication token not found");
        }
    };

    println!(
        "\n{} Cleaning all resources for lab_id: {}\n",
        Emoji::Warning,
        lab_id
    );

    let rpc_client = RpcClient::new(server_url.to_string(), server_connection.clone());

    #[derive(serde::Serialize)]
    struct CleanParams {
        lab_id: String,
    }

    let clean_data: DestroyResponse = rpc_client
        .call(
            "clean",
            CleanParams {
                lab_id: lab_id.to_string(),
            },
            Some(token),
        )
        .await
        .context("Clean RPC call failed")?;

    display_destroy_results(&clean_data)?;

    Ok(())
}
