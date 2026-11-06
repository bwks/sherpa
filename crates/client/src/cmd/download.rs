use std::time::Duration;

use anyhow::{Context, Result, bail};

use shared::data::{ClientConfig, DownloadLabResponse};
use shared::konst::LAB_FILE_NAME;
use shared::util::{Emoji, file_exists, get_cwd, term_msg_surround};

use crate::lab_files::{LabFiles, register_ssh_config, save_lab_files};
use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

/// Download lab files from server and write to the current directory.
#[tracing::instrument(skip_all, fields(%lab_id, force))]
pub async fn download(
    lab_id: &str,
    force: bool,
    server_url: &str,
    config: &ClientConfig,
) -> Result<()> {
    term_msg_surround(&format!("Download Lab Files - {lab_id}"));

    // Check for existing lab files before connecting to server
    let cwd = get_cwd().context("Failed to determine working directory")?;
    let lab_info_path = format!("{cwd}/{LAB_FILE_NAME}");
    if file_exists(&lab_info_path) && !force {
        bail!(
            "Lab files already exist in this directory ({LAB_FILE_NAME}).\nUse --force to overwrite."
        );
    }

    let token = load_token()?;

    let timeout = Duration::from_secs(config.server_connection.timeout_secs);
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        timeout,
        config.server_connection.clone(),
    );

    println!("Connecting to server: {}", server_url);
    let mut rpc_client = ws_client
        .connect()
        .await
        .context("Failed to connect to server")?;

    let request = RpcRequest::new(
        "download",
        serde_json::json!({
            "lab_id": lab_id,
            "token": token,
        }),
    );

    println!("Requesting lab files from server...");
    let response = rpc_client.call(request).await.context("RPC call failed")?;
    rpc_client.close().await.ok();

    let data: DownloadLabResponse = response.into_result("Download failed")?;

    let ssh_config = save_lab_files(&cwd, &LabFiles::from(&data))?;
    register_ssh_config(&ssh_config);

    println!("\n{} Lab files downloaded successfully", Emoji::Success);

    Ok(())
}
