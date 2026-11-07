//! The redeploy operation: load and check, tear the node down, build it again, and save the
//! lab's SSH files.

use std::path::Path;
use std::sync::{Arc, OnceLock, Weak};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use dashmap::DashMap;
use opentelemetry::KeyValue;
use tokio::sync::Mutex;
use tracing::instrument;

use super::container::redeploy_container;
use super::context::{Redeploy, load_redeploy};
use super::teardown::{destroy_node, recreate_vm_networks};
use super::vm::redeploy_vm;
use crate::auth::context::AuthContext;
use crate::auth::policy::{require_lab_owner, require_lab_starter};
use crate::daemon::state::AppState;
use crate::services::guest_account;
use crate::services::lab_operations;
use crate::services::node_resources;
use crate::services::progress::ProgressSender;
use crate::services::ssh_config;
use crate::tls::lab_ca::{self, NodeIdentity};

use shared::data;
use shared::data::{NodeState, RedeployRequest, RedeployResponse, StatusKind};

// Redeploy updates shared lab files. Serialize redeploys within each lab so one
// node's new identity cannot overwrite another node's concurrent update.
static REDEPLOY_LOCKS: OnceLock<DashMap<String, Weak<Mutex<()>>>> = OnceLock::new();

/// Redeploy a single node: destroy existing + recreate with fresh ZTP
///
/// Only the lab owner may redeploy, even for admins: the node is rebuilt with the owner's
/// own SSH keys and account. Authorization is checked before any progress is sent or the
/// lab lock is taken. `request.username` is ignored.
///
/// A node whose rebuild fails after it was torn down is recorded `Failed`.
#[instrument(skip(ctx, state, progress), fields(username = %ctx.username, lab_id = %request.lab_id, node_name = %request.node_name))]
pub async fn redeploy_node(
    ctx: &AuthContext,
    request: RedeployRequest,
    state: &AppState,
    progress: ProgressSender,
) -> Result<RedeployResponse> {
    let start_time = Instant::now();
    let lab_id = request.lab_id.clone();
    let node_name = request.node_name.clone();
    require_lab_starter(ctx)?;
    require_lab_owner(ctx, &state.db, &lab_id).await?;
    let owner = db::get_user(&state.db, &ctx.username)
        .await
        .context("Failed to look up the lab owner")?;
    guest_account::require_ssh_key(&owner)?;
    // Destroying the lab stops this operation; see `lab_operations`.
    let operation = lab_operations::begin(&lab_id).await;
    // Check again under the lease: a destroy that finished while this waited has removed the lab.
    let db_lab = require_lab_owner(ctx, &state.db, &lab_id).await?;
    let lab_lock = lab_lock(&lab_id);
    let _lab_guard = lab_lock.lock().await;

    tracing::info!(
        lab_id = %lab_id,
        node_name = %node_name,
        "Starting node redeploy"
    );

    let mut redeploy = load_redeploy(ctx, request, &db_lab, state, &progress, &operation).await?;

    // Every check has passed, so the node is torn down now. Until then a refused redeploy
    // leaves it, its state and its host key as they were.
    // The rebuilt node gets a new host key, recorded again once it is ready.
    db::update_node_host_key(&state.db, redeploy.node_record_id.clone(), None).await?;
    db::update_node_state(
        &state.db,
        redeploy.node_record_id.clone(),
        NodeState::Starting,
    )
    .await?;
    if let Err(error) = rebuild(&mut redeploy).await {
        mark_failed(&redeploy).await;
        return Err(error);
    }

    operation.ensure_active()?;
    ssh_config::refresh_lab_ssh_files(
        &state.db,
        &redeploy.lab_dir,
        &redeploy.lab_record_id,
        &redeploy.lab_info,
        state.config.server_ipv4,
    )
    .await
    .context("Node redeployed, but failed to save the lab SSH files")?;

    let total_time = start_time.elapsed().as_secs();

    state.metrics.operation_duration.record(
        start_time.elapsed().as_secs_f64(),
        &[KeyValue::new("operation.type", "redeploy")],
    );

    let response = RedeployResponse {
        success: true,
        node_name: node_name.clone(),
        message: format!("Node '{}' redeployed successfully", node_name),
        total_time_secs: total_time,
    };

    tracing::info!(
        lab_id = %lab_id,
        node_name = %node_name,
        total_time_secs = total_time,
        "Node redeploy completed"
    );

    Ok(response)
}

/// The lab's redeploy lock, shared by every redeploy of the lab while one holds it.
fn lab_lock(lab_id: &str) -> Arc<Mutex<()>> {
    let locks = REDEPLOY_LOCKS.get_or_init(DashMap::new);
    locks.retain(|_, lock| lock.strong_count() > 0);
    let mut entry = locks.entry(lab_id.to_owned()).or_default();
    match entry.upgrade() {
        Some(lock) => lock,
        None => {
            let lock = Arc::new(Mutex::new(()));
            *entry = Arc::downgrade(&lock);
            lock
        }
    }
}

/// Tear the node down and build it again (stages 2 to 6).
async fn rebuild(redeploy: &mut Redeploy<'_>) -> Result<()> {
    destroy_node(redeploy).await?;
    if matches!(redeploy.node_image.kind, data::NodeKind::VirtualMachine) {
        recreate_vm_networks(redeploy)?;
    }

    let _ = redeploy.progress.send_status(
        format!("Regenerating ZTP for node: {}", redeploy.node_name),
        StatusKind::Progress,
    );

    // The node keeps its VNC port; its vCPUs and memory follow the manifest.
    redeploy.db_node = node_resources::reserve_node_resources(
        &redeploy.state.db,
        &redeploy.db_node,
        &redeploy.node_image.kind,
        node_resources::planned_resources(&redeploy.target_node, &redeploy.node_image),
    )
    .await?;

    // Reissue the node's certificate from the lab's saved CA, for its current addresses.
    let cert_paths = lab_ca::issue_node_certificate(
        &redeploy.lab_ca,
        Path::new(&redeploy.certs_dir),
        NodeIdentity {
            name: &redeploy.node_name,
            lab_id: &redeploy.lab_id,
            ipv4: redeploy.ipv4,
            ipv6: redeploy.ipv6,
        },
    )?;

    match redeploy.node_image.kind {
        data::NodeKind::Container => redeploy_container(redeploy, &cert_paths).await,
        data::NodeKind::VirtualMachine => redeploy_vm(redeploy, &cert_paths).await,
        data::NodeKind::Unikernel => bail!("Redeploy not supported for unikernel nodes"),
    }
}

/// Record the node as failed, so it is not left `Starting` until the scanner next looks at
/// it. The redeploy's own error is the one returned, so a failure here is only logged.
async fn mark_failed(redeploy: &Redeploy<'_>) {
    if let Err(error) = db::update_node_state(
        &redeploy.state.db,
        redeploy.node_record_id.clone(),
        NodeState::Failed,
    )
    .await
    {
        tracing::warn!(
            lab_id = %redeploy.lab_id,
            node_name = %redeploy.node_name,
            error = %error,
            "Failed to record the node's failed redeploy"
        );
    }
}
