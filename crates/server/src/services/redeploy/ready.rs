//! Waiting for a redeployed node to become ready, on the readiness check stored for it from
//! the manifest it is redeployed from.

use std::time::{Duration, Instant};

use anyhow::Result;
use tracing::instrument;

use super::context::Redeploy;
use crate::konst::READINESS_SLEEP;
use crate::services::node_lifecycle;
use crate::services::node_ops::{self, VmReadiness};

use shared::data::{OperationEvent, ReadyCheck, StatusKind};

/// What a redeployed node waits on before it is recorded running.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ReadyWait {
    /// Its stored port answering.
    Port(u16),
    /// Nothing: its check is skipped.
    Skipped,
    /// Nothing: it has no stored port, so it is ready once it runs.
    Running,
}

/// What a redeployed node waits on, from the check `rebuild` stored for it.
pub(super) fn ready_wait(ready_check: &ReadyCheck) -> ReadyWait {
    match node_ops::probed_ready_port(ready_check) {
        Some(port) => ReadyWait::Port(port),
        None if ready_check.skip => ReadyWait::Skipped,
        None => ReadyWait::Running,
    }
}

/// Wait for the node's stored ready port until the manifest's ready timeout, recording the host
/// key it offers, and record it running: a VM's stage 6, and a container's with a ready port
/// once its services are restored. A node that is not ready in time is left `Starting`, for the
/// scanner to record running once its port answers; the redeploy still succeeds. A skipped
/// check, or a node with no stored port, records it running at once.
#[instrument(skip_all, fields(lab_id = %redeploy.lab_id, node_name = %redeploy.node_name), level = "debug")]
pub(super) async fn wait_until_ready(redeploy: &Redeploy<'_>) -> Result<()> {
    let node_name = redeploy.node_name.as_str();
    let progress = redeploy.progress;
    let _ = progress.send_status(
        format!("Waiting for node {node_name} to become ready"),
        StatusKind::Waiting,
    );

    let ready = match ready_wait(&redeploy.db_node.ready_check) {
        ReadyWait::Port(ready_port) => poll_until_ready(redeploy, ready_port).await?,
        ReadyWait::Skipped => {
            let _ = progress.send_status(
                format!("Node {node_name} - Ready check skipped"),
                StatusKind::Done,
            );
            true
        }
        ReadyWait::Running => true,
    };

    if ready {
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

    while start.elapsed() < redeploy.ready_timeout() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::{ModelKind, NodeModel};
    use shared::konst::SSH_PORT;
    use topology::{NodeExpanded, NodeReadyCheck};

    use crate::services::node_lifecycle::tests::lab_of;

    fn stored(port: Option<u16>, skip: bool) -> ReadyCheck {
        ReadyCheck { port, skip }
    }

    #[test]
    fn a_redeployed_node_waits_on_its_stored_check() {
        assert_eq!(
            ready_wait(&stored(Some(8443), false)),
            ReadyWait::Port(8443)
        );
        assert_eq!(
            ready_wait(&stored(Some(SSH_PORT), true)),
            ReadyWait::Skipped
        );
        assert_eq!(ready_wait(&stored(None, true)), ReadyWait::Skipped);
        assert_eq!(ready_wait(&stored(None, false)), ReadyWait::Running);
    }

    #[tokio::test]
    async fn the_new_manifests_check_replaces_the_stored_one_before_the_wait() {
        // Stored by `up`: the container had no port.
        let (db, _, nodes) = lab_of("rd-redy1", 1).await;
        let node = db::get_node(&db, nodes[0].clone()).await.unwrap();
        assert_eq!(ready_wait(&node.ready_check), ReadyWait::Running);

        // The manifest it is redeployed from gives it a port, then skips it.
        for (manifest, wait) in [
            (
                NodeReadyCheck {
                    port: Some(8080),
                    skip: None,
                },
                ReadyWait::Port(8080),
            ),
            (
                NodeReadyCheck {
                    port: Some(8080),
                    skip: Some(true),
                },
                ReadyWait::Skipped,
            ),
        ] {
            let target = NodeExpanded {
                model_kind: ModelKind::default_for(NodeModel::GenericContainer),
                ready_check: manifest,
                ..NodeExpanded::default()
            };
            let record = db::update_node_ready_check(
                &db,
                nodes[0].clone(),
                &node_ops::stored_ready_check(target.model_kind, &target.ready_check),
            )
            .await
            .unwrap();
            assert_eq!(ready_wait(&record.ready_check), wait);
            let read = db::get_node(&db, nodes[0].clone()).await.unwrap();
            assert_eq!(read.ready_check, record.ready_check);
        }
    }
}
