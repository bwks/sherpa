use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use shared::data::{ClientConfig, DownloadLabResponse};
use shared::error::RpcErrorCode;
use shared::konst::{LAB_FILE_NAME, SHERPA_SSH_CONFIG_FILE, SHERPA_SSH_PRIVATE_KEY_FILE};
use shared::util::{Emoji, add_lab_ssh_include, file_exists, get_cwd, term_msg_surround};

use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

/// Rewrite server-generated SSH config for client-side use.
///
/// Replaces relative `IdentityFile sherpa_ssh_key` with absolute path using the working directory.
fn rewrite_ssh_config_for_client(ssh_config: &str, cwd: &str) -> String {
    let abs_key_path = Path::new(cwd)
        .join(SHERPA_SSH_PRIVATE_KEY_FILE)
        .to_string_lossy()
        .to_string();

    let result = ssh_config.replace(
        &format!("IdentityFile {}", SHERPA_SSH_PRIVATE_KEY_FILE),
        &format!("IdentityFile {}", abs_key_path),
    );

    #[cfg(windows)]
    let result = result.replace("UserKnownHostsFile /dev/null", "UserKnownHostsFile NUL");

    result
}

/// Download lab files from server and write to the current directory.
pub async fn download(
    lab_id: &str,
    force: bool,
    server_url: &str,
    config: &ClientConfig,
) -> Result<()> {
    term_msg_surround(&format!("Download Lab Files - {lab_id}"));

    // Check for existing lab files before connecting to server
    let cwd = get_cwd().context("Failed to determine working directory")?;
    let lab_info_path = format!("{}/{}", cwd, LAB_FILE_NAME);
    if file_exists(&lab_info_path) && !force {
        bail!(
            "Lab files already exist in this directory ({}).\nUse --force to overwrite.",
            LAB_FILE_NAME
        );
    }

    let token = match load_token() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("\n{} Authentication required", Emoji::Error);
            eprintln!("   Please run: sherpa login");
            eprintln!("   Error: {}", e);
            bail!("Authentication token not found");
        }
    };

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

    if let Some(error) = response.error {
        eprintln!("\n{} Server Error:", Emoji::Error);
        eprintln!("   Message: {}", error.message);
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
        bail!("Download failed");
    }

    let result = response.result.context("No result in response")?;
    let data: DownloadLabResponse =
        serde_json::from_value(result).context("Failed to parse download response")?;

    // Write SSH config with absolute IdentityFile path
    let ssh_config = rewrite_ssh_config_for_client(&data.ssh_config, &cwd);
    let ssh_config_path = Path::new(&cwd)
        .join(SHERPA_SSH_CONFIG_FILE)
        .to_string_lossy()
        .to_string();
    fs::write(&ssh_config_path, &ssh_config).context("Failed to write SSH config")?;
    println!("{} SSH config created: {}", Emoji::Success, ssh_config_path);

    match add_lab_ssh_include(&ssh_config_path) {
        Ok(_) => {
            println!(
                "{} SSH config registered in ~/.ssh/sherpa_lab_hosts",
                Emoji::Success,
            );
        }
        Err(e) => {
            println!(
                "\n{} Warning: Failed to register SSH config: {}",
                Emoji::Warning,
                e
            );
        }
    }

    // Write SSH private key with 0600 permissions
    let ssh_key_path = Path::new(&cwd)
        .join(SHERPA_SSH_PRIVATE_KEY_FILE)
        .to_string_lossy()
        .to_string();
    fs::write(&ssh_key_path, &data.ssh_private_key).context("Failed to write SSH key")?;

    #[cfg(unix)]
    {
        if let Err(e) = fs::set_permissions(&ssh_key_path, fs::Permissions::from_mode(0o600)) {
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
        ssh_key_path
    );

    // Write lab-info.toml
    fs::write(&lab_info_path, data.lab_info.to_string())
        .context("Failed to write lab-info.toml")?;
    println!("{} Lab info created: {}", Emoji::Success, lab_info_path);

    println!("\n{} Lab files downloaded successfully", Emoji::Success);

    Ok(())
}
