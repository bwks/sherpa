//! Waiting for a redeployed node to answer on its ready port, for VMs and for containers with
//! a `ready_check.port`.

use std::time::{Duration, Instant};

use anyhow::Result;
use tracing::instrument;

use super::context::Redeploy;
use crate::services::node_lifecycle;
use crate::services::node_ops::{self, VmReadiness};

use shared::data::{OperationEvent, StatusKind};
use shared::konst::READINESS_SLEEP;

/// Wait for the node's `ready_port` until the manifest's ready timeout, recording the host key
/// it offers, and record it running: a VM's stage 6, and a container's with a
/// `ready_check.port` once its services are restored. A node that is not ready in time is left
/// `Starting`, for the scanner to record running; the redeploy still succeeds. A skipped check
/// records it running at once.
#[instrument(skip_all, fields(lab_id = %redeploy.lab_id, node_name = %redeploy.node_name), level = "debug")]
pub(super) async fn wait_until_ready(redeploy: &Redeploy<'_>, ready_port: u16) -> Result<()> {
    let node_name = redeploy.node_name.as_str();
    let progress = redeploy.progress;
    let _ = progress.send_status(
        format!("Waiting for node {node_name} to become ready"),
        StatusKind::Waiting,
    );

    let skip_ready = redeploy.target_node.ready_check.skip.unwrap_or(false);
    if skip_ready {
        let _ = progress.send_status(
            format!("Node {node_name} - Ready check skipped"),
            StatusKind::Done,
        );
    }

    if skip_ready || poll_until_ready(redeploy, ready_port).await? {
        node_lifecycle::record(
            &redeploy.state.db,
            redeploy.operation,
            &redeploy.node_record_id,
            OperationEvent::Ready,
        )
        .await?;
    } else {
        let _ = progress.send_status(
            format!("Node {node_name} did not become ready within timeout"),
            StatusKind::Waiting,
        );
    }
    Ok(())
}

/// Poll the node's `ready_port` on its management address, as `up` checks a VM's, until it
/// answers or the ready timeout passes. Returns whether it answered, and records the host key
/// it offers.
async fn poll_until_ready(redeploy: &Redeploy<'_>, ready_port: u16) -> Result<bool> {
    let node_name = redeploy.node_name.as_str();
    let ready_label = node_ops::ready_port_label(ready_port);
    let ip = redeploy.ipv4.to_string();
    let start = Instant::now();

    while start.elapsed() < redeploy.ready_timeout {
        match node_ops::check_vm_ready(&ip, ready_port).await? {
            VmReadiness::Ready { host_key } => {
                if host_key.is_some() {
                    db::update_node_host_key(
                        &redeploy.state.db,
                        redeploy.node_record_id.clone(),
                        host_key.as_deref(),
                    )
                    .await?;
                }
                let _ = redeploy.progress.send_status(
                    format!("Node {node_name} - Ready ({ready_label} reachable)"),
                    StatusKind::Done,
                );
                return Ok(true);
            }
            VmReadiness::NotReady => {
                let _ = redeploy.progress.send_status(
                    format!("Node {node_name} - Waiting for {ready_label}"),
                    StatusKind::Waiting,
                );
                redeploy
                    .operation
                    .sleep(Duration::from_secs(READINESS_SLEEP))
                    .await?;
            }
        }
    }
    Ok(false)
}
