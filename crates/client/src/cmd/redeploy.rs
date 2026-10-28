use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use shared::data::{ClientConfig, RedeployResponse, StatusKind, StatusMessage};
use shared::error::RpcErrorCode;
use shared::util::{Emoji, term_msg_surround};
use topology::validate::validate_provisioners;

use crate::cmd::interrupt::{
    OperationTarget, StoppableOperation, Waited, call_streaming_or_interrupt,
};
use crate::provision::{container_results, node_secrets, provision_lab};
use crate::ssh_client::HostKeyRefresh;
use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

use super::download::download;
use super::up::resolve_manifest_files;

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

    // Load the manifest and resolve its client files, as `up` does.
    let mut manifest = topology::Manifest::load_file(manifest_path)?;
    validate_provisioners(&manifest).context("Manifest validation failed: provisioners")?;
    resolve_manifest_files(&mut manifest, manifest_path)?;

    // Always sent, so a node whose container provisioner was removed loses its secret files.
    let manifest_dir = Path::new(manifest_path).parent().unwrap_or(Path::new("."));
    let mut secrets = Vec::new();
    for node in &manifest.nodes {
        if node.name == node_name {
            secrets = node_secrets(node, manifest_dir)?;
        }
    }

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
            "secrets": secrets,
            "token": token,
        }),
    );

    let target = OperationTarget {
        operation: StoppableOperation::Redeploy,
        lab_id,
        token: &token,
        server_url,
        server_connection: &config.server_connection,
    };
    let redeploy_response = match call_streaming_or_interrupt(
        &mut rpc_client,
        redeploy_request,
        |msg_text| {
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
        },
        &target,
    )
    .await
    .context("Redeploy RPC call failed")?
    {
        Waited::Response(response) => response,
        Waited::Interrupted => bail!("Redeploy interrupted"),
    };

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

    download(lab_id, true, server_url, config)
        .await
        .with_context(|| format!("Node redeployed, but local SSH configuration refresh failed; run sherpa download {lab_id} --force"))?;

    let mut mounted = BTreeMap::new();
    if !secrets.is_empty() {
        mounted.insert(node_name.to_string(), secrets);
    }
    provision_lab(
        &manifest,
        manifest_dir,
        lab_id,
        None,
        Some(node_name),
        container_results(&mounted),
        &HostKeyRefresh::new(lab_id, server_url, config, manifest_dir),
    )
    .await?;

    Ok(())
}
