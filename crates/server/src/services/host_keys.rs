//! Reading a lab node's SSH host key again on request, for a client whose recorded key does
//! not match the one the node offered.

use anyhow::{Context, Result};
use tracing::instrument;

use shared::data::{NodeHostKeysRequest, NodeHostKeysResponse, NodeKind};
use shared::konst::{SHERPA_LABS_PATH, SSH_PORT};

use crate::auth::context::AuthContext;
use crate::auth::policy::require_lab_access;
use crate::daemon::state::AppState;
use crate::services::error::ServiceError;
use crate::services::node_ops::{self, VmReadiness};
use crate::services::ssh_config;

/// Read the node's host key, record it if it changed, and return the lab's `known_hosts`
/// file. A node whose SSH server does not answer keeps its recorded key.
///
/// The caller must own the lab or be an admin.
#[instrument(skip(ctx, request, state), fields(username = %ctx.username, lab_id = %request.lab_id, node_name = %request.node_name))]
pub async fn refresh_node_host_key(
    ctx: &AuthContext,
    request: &NodeHostKeysRequest,
    state: &AppState,
) -> Result<NodeHostKeysResponse> {
    let lab_id = &request.lab_id;
    let node_name = &request.node_name;
    let db_lab = require_lab_access(ctx, &state.db, lab_id).await?;
    let lab_record_id = db::get_lab_id(&db_lab).context("Failed to get lab record ID")?;
    let db_node =
        match db::get_node_by_name_and_lab(&state.db, node_name, lab_record_id.clone()).await {
            Ok(db_node) => db_node,
            Err(error) => {
                // The lookup returns one error for a missing node and a failed query.
                tracing::warn!(error = %format!("{error:#}"), "Node lookup failed");
                return Err(ServiceError::NotFound(format!(
                    "Node '{node_name}' not found in lab '{lab_id}'"
                ))
                .into());
            }
        };
    let node_images = db::list_node_images_by_ids(&state.db, vec![db_node.image.clone()])
        .await
        .context("Failed to get node image config")?;
    let node_image = node_images
        .first()
        .with_context(|| format!("Node image config not found for node '{node_name}'"))?;
    if node_image.kind != NodeKind::VirtualMachine || !node_image.model.has_ssh() {
        return Err(ServiceError::Conflict(format!(
            "Node '{node_name}' is not a virtual machine with SSH, so it has no recorded host key"
        ))
        .into());
    }
    let ip = db_node
        .mgmt_ipv4
        .as_deref()
        .with_context(|| format!("Node '{node_name}' has no management address"))?;

    let changed = match node_ops::check_vm_ready(ip, SSH_PORT).await? {
        VmReadiness::Ready {
            host_key: Some(host_key),
        } => ssh_config::record_node_host_key(&state.db, &db_node, &host_key).await?,
        VmReadiness::Ready { host_key: None } | VmReadiness::NotReady => {
            tracing::info!("Node SSH server not answering; keeping the recorded key");
            false
        }
    };

    let lab_dir = format!("{SHERPA_LABS_PATH}/{lab_id}");
    let lab_info = ssh_config::load_lab_info(&lab_dir).await?;
    let files = ssh_config::refresh_lab_ssh_files(
        &state.db,
        &lab_dir,
        &lab_record_id,
        &lab_info,
        state.config.server_ipv4,
    )
    .await?;
    Ok(NodeHostKeysResponse {
        known_hosts: files.known_hosts,
        changed,
    })
}
