use anyhow::{Context, Result};

use shared::data::{BackupResponse, ServerConnection};
use shared::util::{render_backup_table, term_msg_surround};

use super::{OutputFormat, rpc_call};

/// Back up the server's database to a file on the server (admin-only).
pub async fn backup(
    server_url: &str,
    server_connection: &ServerConnection,
    output: &OutputFormat,
) -> Result<()> {
    // An empty object: the token is added to the params, so they must be an object.
    let params = serde_json::Map::new();
    let response: BackupResponse = rpc_call("server.backup", params, server_url, server_connection)
        .await
        .context("Backup RPC call failed")?;

    match output {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&response)
                    .context("Failed to serialize the backup response")?
            );
        }
        OutputFormat::Text => {
            term_msg_surround("Database backup");
            println!("{}", render_backup_table(&response));
        }
    }
    Ok(())
}
