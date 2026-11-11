use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use shared::data::{ClientConfig, RedeployResponse};
use shared::util::{Emoji, term_msg_surround};
use topology::validate::{check_text_file_modes, validate_provisioners};

use crate::cmd::interrupt::{
    OperationTarget, StoppableOperation, Waited, call_streaming_or_interrupt,
};
use crate::provision::{container_results, node_secrets, provision_lab};
use crate::ssh_client::HostKeyRefresh;
use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

use super::download::download;
use super::outcome::print_link_reports;
use super::progress::print_status;
use super::up::resolve_manifest_files;

/// Redeploy a single node: destroy and recreate with fresh ZTP
#[tracing::instrument(skip_all, fields(%lab_name, %lab_id, %node_name))]
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

    let token = load_token()?;

    // Load the manifest and resolve its client files, as `up` does.
    let mut manifest = topology::Manifest::load_file(manifest_path)?;
    validate_provisioners(&manifest).context("Manifest validation failed: provisioners")?;
    check_text_file_modes(&manifest.nodes).context("Manifest validation failed: text files")?;
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

    // Limits the connect only: the operation streams its progress for as long as it runs.
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
    let redeploy_response =
        match call_streaming_or_interrupt(&mut rpc_client, redeploy_request, print_status, &target)
            .await
            .context("Redeploy RPC call failed")?
        {
            Waited::Response(response) => response,
            Waited::Interrupted => bail!("Redeploy interrupted"),
        };

    rpc_client.close().await.ok();

    // Handle errors
    let result: RedeployResponse = redeploy_response.into_result("Redeploy operation failed")?;
    if !result.success {
        bail!(
            "Node '{}' was not redeployed: {}",
            result.node_name,
            result.message
        );
    }

    println!(
        "\n{} Node '{}' redeployed successfully in {}s",
        Emoji::Success,
        result.node_name,
        result.total_time_secs
    );
    print_link_reports(&result.links);

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
