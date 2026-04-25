use std::collections::HashSet;
use std::time::Duration;

use anyhow::{Context, Result};
use tracing::instrument;

use shared::data::{ClientConfig, ListLabsResponse};
use shared::util::{
    Emoji, SshConfigEntryStatus, SshConfigInspectionEntry, clean_stale_lab_ssh_includes,
    inspect_lab_ssh_includes, render_ssh_config_inspection_table, term_msg_surround,
};

use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

const LABS_LIST_RPC_METHOD: &str = "labs.list";

struct ServerValidation {
    active_lab_ids: Option<HashSet<String>>,
    message: Option<String>,
}

impl ServerValidation {
    fn available(active_lab_ids: HashSet<String>) -> Self {
        Self {
            active_lab_ids: Some(active_lab_ids),
            message: None,
        }
    }

    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            active_lab_ids: None,
            message: Some(message.into()),
        }
    }
}

async fn fetch_active_lab_ids(server_url: &str, config: &ClientConfig) -> ServerValidation {
    let token = match load_token() {
        Ok(token) => token,
        Err(e) => {
            return ServerValidation::unavailable(format!(
                "Authentication token unavailable; server validation skipped: {e}"
            ));
        }
    };

    let timeout = Duration::from_secs(config.server_connection.timeout_secs);
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        timeout,
        config.server_connection.clone(),
    );

    let mut rpc_client = match ws_client.connect().await {
        Ok(client) => client,
        Err(e) => {
            return ServerValidation::unavailable(format!(
                "Could not connect to server; server validation skipped: {e}"
            ));
        }
    };

    let request = RpcRequest::new(
        LABS_LIST_RPC_METHOD,
        serde_json::json!({
            "token": token,
        }),
    );

    let response = match rpc_client.call(request).await {
        Ok(response) => response,
        Err(e) => {
            rpc_client.close().await.ok();
            return ServerValidation::unavailable(format!(
                "Could not list labs from server; server validation skipped: {e}"
            ));
        }
    };
    rpc_client.close().await.ok();

    if let Some(error) = response.error {
        return ServerValidation::unavailable(format!(
            "Server rejected lab listing; server validation skipped: {}",
            error.message
        ));
    }

    let Some(result) = response.result else {
        return ServerValidation::unavailable(
            "Server returned no lab listing; server validation skipped",
        );
    };

    match serde_json::from_value::<ListLabsResponse>(result) {
        Ok(list) => ServerValidation::available(list.labs.into_iter().map(|lab| lab.id).collect()),
        Err(e) => ServerValidation::unavailable(format!(
            "Could not parse lab listing; server validation skipped: {e}"
        )),
    }
}

fn entry_word(count: usize) -> &'static str {
    if count == 1 { "entry" } else { "entries" }
}

fn print_server_validation_warning(validation: &ServerValidation) {
    if let Some(message) = &validation.message {
        println!("{} {}", Emoji::Warning, message);
    }
}

fn count_entries_with_status(
    entries: &[SshConfigInspectionEntry],
    status: SshConfigEntryStatus,
) -> usize {
    entries
        .iter()
        .filter(|entry| entry.status == status)
        .count()
}

#[instrument(skip(config), fields(%server_url))]
pub async fn ssh_config_inspect(server_url: &str, config: &ClientConfig) -> Result<()> {
    term_msg_surround("Inspect Sherpa SSH config includes");

    let validation = fetch_active_lab_ids(server_url, config).await;
    print_server_validation_warning(&validation);

    let report = inspect_lab_ssh_includes(
        validation.active_lab_ids.as_ref(),
        validation.message.clone(),
    )
    .context("Failed to inspect Sherpa SSH config includes")?;

    println!("\nIndex file: {}", report.index_path.display());
    if report.entries.is_empty() {
        println!("{} No Sherpa SSH config Include entries found", Emoji::Info);
        return Ok(());
    }

    println!("\n{}", render_ssh_config_inspection_table(&report.entries));

    let stale_count = count_entries_with_status(&report.entries, SshConfigEntryStatus::Stale);
    let broken_count = count_entries_with_status(&report.entries, SshConfigEntryStatus::Broken);
    let stale_or_broken_count = stale_count + broken_count;

    if stale_or_broken_count > 0 {
        println!(
            "\n{} Found {} stale and {} broken SSH config {}",
            Emoji::Warning,
            stale_count,
            broken_count,
            entry_word(stale_or_broken_count),
        );
    } else {
        println!("\n{} No stale SSH config entries found", Emoji::Success);
    }

    Ok(())
}

#[instrument(skip(config), fields(%server_url))]
pub async fn ssh_config_clean(server_url: &str, config: &ClientConfig) -> Result<()> {
    term_msg_surround("Clean Sherpa SSH config includes");

    let validation = fetch_active_lab_ids(server_url, config).await;
    print_server_validation_warning(&validation);

    let report = clean_stale_lab_ssh_includes(
        validation.active_lab_ids.as_ref(),
        validation.message.clone(),
    )
    .context("Failed to clean Sherpa SSH config includes")?;

    println!("\nIndex file: {}", report.index_path.display());

    if report.removed.is_empty() {
        println!(
            "{} No stale or broken SSH config entries removed",
            Emoji::Info
        );
    } else {
        println!(
            "{} Removed {} stale/broken SSH config {}",
            Emoji::Success,
            report.removed.len(),
            entry_word(report.removed.len()),
        );
        println!("\n{}", render_ssh_config_inspection_table(&report.removed));
    }

    if !report.kept.is_empty() {
        println!(
            "\n{} Kept {} SSH config {}",
            Emoji::Info,
            report.kept.len(),
            entry_word(report.kept.len()),
        );
    }

    Ok(())
}
