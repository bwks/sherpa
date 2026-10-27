use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};

use shared::data::{ClientConfig, NodeSecretsResponse, ProvisionResult, ProvisionStatus};
use shared::konst::{CONTAINER_SECRETS_DIR, SHERPA_MANIFEST_FILE};
use shared::util::{get_id, term_msg_surround};
use topology::Manifest;
use topology::validate::validate_provisioners;

use crate::provision::{container_secrets, provision_lab};
use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

/// Run the lab's node provisioners again, from the lab folder: the server rewrites container
/// nodes' secret files, and the CLI runs ssh provisioners.
pub async fn provision(node: Option<&str>, server_url: &str, config: &ClientConfig) -> Result<()> {
    let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
    let lab_id = get_id(&manifest.name)?;
    term_msg_surround(&format!("Provision Lab - {}-{lab_id}", manifest.name));
    validate_provisioners(&manifest).context("Manifest validation failed: provisioners")?;

    let manifest_dir = Path::new(SHERPA_MANIFEST_FILE)
        .parent()
        .unwrap_or(Path::new("."));
    let mut secrets = container_secrets(&manifest, manifest_dir)?;
    if let Some(only) = node {
        let mut kept = BTreeMap::new();
        if let Some(files) = secrets.remove(only) {
            kept.insert(only.to_string(), files);
        }
        secrets = kept;
    }

    let mut results = Vec::with_capacity(secrets.len());
    if !secrets.is_empty() {
        let token = load_token().context("Authentication required: run sherpa login")?;
        let ws_client = WebSocketClient::new(
            server_url.to_string(),
            Duration::from_secs(config.server_connection.timeout_secs),
            config.server_connection.clone(),
        );
        let mut rpc_client = ws_client
            .connect()
            .await
            .context("Failed to connect to sherpad server")?;
        for (node_name, files) in secrets {
            println!("Writing secret files for {node_name}.{lab_id}");
            let request = RpcRequest::new(
                "node.secrets",
                serde_json::json!({
                    "lab_id": lab_id,
                    "node_name": node_name,
                    "files": files,
                    "token": token,
                }),
            );
            let result = match rpc_client.call(request).await {
                Ok(response) => match (response.error, response.result) {
                    (Some(error), _) => failed(&node_name, error.message),
                    (None, Some(value)) => {
                        match serde_json::from_value::<NodeSecretsResponse>(value) {
                            Ok(written) => ProvisionResult {
                                node: node_name,
                                status: ProvisionStatus::Provisioned,
                                detail: format!(
                                    "{} secret files in {CONTAINER_SECRETS_DIR}",
                                    written.files_written
                                ),
                            },
                            Err(error) => {
                                failed(&node_name, format!("Invalid server response: {error}"))
                            }
                        }
                    }
                    (None, None) => failed(&node_name, "Empty server response".to_string()),
                },
                Err(error) => failed(&node_name, format!("{error:#}")),
            };
            results.push(result);
        }
        rpc_client.close().await.ok();
    }

    let provisioned = provision_lab(&manifest, manifest_dir, &lab_id, None, node, results).await?;
    if provisioned == 0 {
        println!("No provisioners to run");
    }
    Ok(())
}

fn failed(node: &str, detail: String) -> ProvisionResult {
    ProvisionResult {
        node: node.to_string(),
        status: ProvisionStatus::Failed,
        detail,
    }
}
