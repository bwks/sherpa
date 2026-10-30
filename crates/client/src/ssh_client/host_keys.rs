//! Fetching a lab's node host keys from the Sherpa server when a node's key does not match
//! the lab's `known_hosts` file, as after the node was restarted from the web UI.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::sync::Mutex;

use shared::data::{ClientConfig, NodeHostKeysResponse, ServerConnection};

use crate::lab_files::write_known_hosts;
use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

/// Where to ask for a lab's node host keys, and the lab folder their file is saved in.
/// Clones share one lock, so nodes provisioned at once refresh the file one at a time.
#[derive(Clone, Debug)]
pub struct HostKeyRefresh {
    lab_id: String,
    server_url: String,
    server_connection: ServerConnection,
    lab_dir: PathBuf,
    refreshing: Arc<Mutex<()>>,
}

impl HostKeyRefresh {
    pub fn new(lab_id: &str, server_url: &str, config: &ClientConfig, lab_dir: &Path) -> Self {
        Self {
            lab_id: lab_id.to_owned(),
            server_url: server_url.to_owned(),
            server_connection: config.server_connection.clone(),
            lab_dir: lab_dir.to_path_buf(),
            refreshing: Arc::new(Mutex::new(())),
        }
    }

    /// Ask the server to read `node_name`'s host key again, and save the lab's `known_hosts`
    /// file it returns in the lab folder. One refresh runs at a time, so two never write the
    /// file at once.
    #[tracing::instrument(skip(self), fields(lab_id = %self.lab_id), level = "debug")]
    pub async fn refresh(&self, node_name: &str) -> Result<NodeHostKeysResponse> {
        let _turn = self.refreshing.lock().await;
        let token = load_token().context("Authentication required: run sherpa login")?;
        let ws_client = WebSocketClient::new(
            self.server_url.clone(),
            Duration::from_secs(self.server_connection.timeout_secs),
            self.server_connection.clone(),
        );
        let mut rpc_client = ws_client
            .connect()
            .await
            .context("Failed to connect to sherpad server")?;
        let request = RpcRequest::new(
            "node.host_keys",
            serde_json::json!({
                "lab_id": self.lab_id,
                "node_name": node_name,
                "token": token,
            }),
        );
        let response = rpc_client.call(request).await;
        rpc_client.close().await.ok();
        let response = response.context("RPC call failed")?;
        if let Some(error) = response.error {
            bail!(
                "The Sherpa server could not read the host key: {}",
                error.message
            );
        }
        let result = response
            .result
            .context("Empty response from the Sherpa server")?;
        let keys: NodeHostKeysResponse =
            serde_json::from_value(result).context("Invalid host key response")?;
        let lab_dir = self.lab_dir.to_string_lossy();
        write_known_hosts(&lab_dir, &keys.known_hosts)?;
        Ok(keys)
    }
}
