use anyhow::{Context, Result, anyhow};
use futures::future::join_all;
use opentelemetry::KeyValue;
use shared::data::{self, LabNodeActionResponse, NodeActionResult, NodeKind, NodeState, RecordId};
use std::time::Instant;
use virt::sys::{VIR_DOMAIN_PAUSED, VIR_DOMAIN_SHUTOFF};

use tracing::instrument;

use crate::daemon::state::AppState;

/// Start/poweron all (or a specific) node(s) for a lab.
///
/// VMs: if shutoff, calls `domain.create()` (cold boot). If paused, calls `domain.resume()`.
/// Containers: calls `docker start`.
/// DB node state is updated to `Running` on success.
#[instrument(skip(state), fields(%lab_id))]
pub async fn start_lab_nodes(
    lab_id: &str,
    node_name: Option<&str>,
    state: &AppState,
) -> Result<LabNodeActionResponse> {
    let start = Instant::now();

    // Get lab from DB to obtain RecordId
    let db_lab = db::get_lab(&state.db, lab_id)
        .await
        .context(format!("Lab '{}' not found in database", lab_id))?;

    let lab_record_id = db_lab.id.ok_or_else(|| anyhow!("Lab missing record ID"))?;

    // Get all nodes for this lab
    let db_nodes = db::list_nodes_by_lab(&state.db, lab_record_id.clone())
        .await
        .context("Failed to list nodes for lab")?;

    // Filter by node_name if provided
    let target_nodes: Vec<_> = if let Some(name) = node_name {
        db_nodes.into_iter().filter(|n| n.name == name).collect()
    } else {
        db_nodes
    };

    if let Some(name) = node_name
        && target_nodes.is_empty()
    {
        return Err(anyhow!("Node '{}' not found in lab '{}'", name, lab_id));
    }

    // Batch-fetch node images to determine kind (VM vs Container)
    let mut image_ids: Vec<RecordId> = target_nodes.iter().map(|n| n.image.clone()).collect();
    image_ids.dedup();

    let node_images = db::list_node_images_by_ids(&state.db, image_ids)
        .await
        .context("Failed to batch fetch node images")?;

    // Build futures for all node operations concurrently
    let mut futures = Vec::new();
    let mut immediate_results = Vec::new();

    for node in &target_nodes {
        let device_name = format!("{}-{}", node.name, lab_id);

        // Determine node kind from image
        let node_image = node_images
            .iter()
            .find(|img| img.id.as_ref() == Some(&node.image));

        let kind = match node_image {
            Some(img) => img.kind.clone(),
            None => {
                immediate_results.push(NodeActionResult {
                    name: node.name.clone(),
                    success: false,
                    message: "Node image not found in database".to_string(),
                });
                continue;
            }
        };

        let node_name = node.name.clone();
        let node_id = node.id.clone();
        let state = state.clone();

        let kind_clone = kind.clone();
        futures.push(async move {
            let result = match kind {
                NodeKind::VirtualMachine | NodeKind::Unikernel => {
                    start_vm(&device_name, &node_name, &state).await
                }
                NodeKind::Container => start_container_node(&device_name, &node_name, &state).await,
            };

            (result, node_name, node_id, kind_clone)
        });
    }

    // Run all operations concurrently
    let concurrent_results = join_all(futures).await;

    // Collect results and update DB state
    let mut results = immediate_results;
    let mut cold_booted_vms: Vec<String> = vec![];
    for (result, node_name, node_id, kind) in concurrent_results {
        if result.success
            && let Some(id) = node_id
            && let Err(e) = db::update_node_state(&state.db, id, NodeState::Running).await
        {
            tracing::warn!("Failed to update DB state for node '{}': {}", node_name, e);
        }
        // Track VMs that were cold-booted (new tap devices with new ifindexes)
        if result.success && result.message == "Started" && matches!(kind, NodeKind::VirtualMachine)
        {
            cold_booted_vms.push(node_name.clone());
        }
        results.push(result);
    }

    // Re-attach eBPF redirect on P2p links for cold-booted VMs.
    // Cold boot creates new tap devices, so the eBPF programs need re-attaching
    // on both sides (new VM tap + peer's stale redirect).
    if !cold_booted_vms.is_empty()
        && let Err(e) =
            reattach_p2p_ebpf_for_nodes(lab_id, &cold_booted_vms, &lab_record_id, state).await
    {
        tracing::error!(
            lab_id = %lab_id,
            error = ?e,
            "Failed to re-attach eBPF redirect after VM cold boot"
        );
    }

    state.metrics.operation_duration.record(
        start.elapsed().as_secs_f64(),
        &[KeyValue::new("operation.type", "resume")],
    );

    Ok(LabNodeActionResponse { results })
}

async fn start_vm(device_name: &str, node_name: &str, state: &AppState) -> NodeActionResult {
    let qemu = state.qemu.clone();
    let device_name = device_name.to_string();
    let node_name_owned = node_name.to_string();

    match tokio::task::spawn_blocking(move || -> Result<NodeActionResult> {
        let node_name = node_name_owned;
        let conn = qemu.connect().context("Failed to connect to libvirt")?;
        let domain = match virt::domain::Domain::lookup_by_name(&conn, &device_name) {
            Ok(d) => d,
            Err(e) => {
                return Ok(NodeActionResult {
                    name: node_name,
                    success: false,
                    message: format!("Domain not found: {}", e),
                });
            }
        };

        match domain.get_state() {
            Ok((state, _reason)) => {
                if state == VIR_DOMAIN_SHUTOFF {
                    // Cold boot a defined-but-inactive domain
                    match domain.create() {
                        Ok(_) => Ok(NodeActionResult {
                            name: node_name,
                            success: true,
                            message: "Started".to_string(),
                        }),
                        Err(e) => Ok(NodeActionResult {
                            name: node_name,
                            success: false,
                            message: format!("Failed to start: {}", e),
                        }),
                    }
                } else if state == VIR_DOMAIN_PAUSED {
                    // Resume a paused domain
                    match domain.resume() {
                        Ok(_) => Ok(NodeActionResult {
                            name: node_name,
                            success: true,
                            message: "Resumed".to_string(),
                        }),
                        Err(e) => Ok(NodeActionResult {
                            name: node_name,
                            success: false,
                            message: format!("Failed to resume: {}", e),
                        }),
                    }
                } else {
                    // Already running or other state
                    Ok(NodeActionResult {
                        name: node_name,
                        success: true,
                        message: "Already running".to_string(),
                    })
                }
            }
            Err(e) => Ok(NodeActionResult {
                name: node_name,
                success: false,
                message: format!("Failed to get state: {}", e),
            }),
        }
    })
    .await
    {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => NodeActionResult {
            name: node_name.to_string(),
            success: false,
            message: format!("Libvirt error: {}", e),
        },
        Err(e) => NodeActionResult {
            name: node_name.to_string(),
            success: false,
            message: format!("Task panicked: {}", e),
        },
    }
}

async fn start_container_node(
    device_name: &str,
    node_name: &str,
    state: &AppState,
) -> NodeActionResult {
    match container::unpause_container(&state.docker, device_name).await {
        Ok(()) => NodeActionResult {
            name: node_name.to_string(),
            success: true,
            message: "Unpaused".to_string(),
        },
        Err(e) => NodeActionResult {
            name: node_name.to_string(),
            success: false,
            message: format!("Failed to unpause: {:#}", e),
        },
    }
}

/// Re-attach eBPF redirect programs on P2p links after VM cold boot.
///
/// When a VM is cold-booted, libvirt creates new tap devices with new ifindexes.
/// The peer side's eBPF program still points to the old (stale) ifindex.
/// This function re-attaches eBPF on both sides of each affected P2p link.
async fn reattach_p2p_ebpf_for_nodes(
    lab_id: &str,
    node_names: &[String],
    lab_record_id: &RecordId,
    state: &AppState,
) -> Result<()> {
    let db_links = db::list_links_by_lab(&state.db, lab_record_id.clone()).await?;
    let db_nodes = db::list_nodes_by_lab(&state.db, lab_record_id.clone()).await?;

    // Build node name -> RecordId lookup
    let node_record_ids: std::collections::HashMap<String, RecordId> = db_nodes
        .iter()
        .filter_map(|n| n.id.clone().map(|id| (n.name.clone(), id)))
        .collect();

    let target_record_ids: Vec<&RecordId> = node_names
        .iter()
        .filter_map(|name| node_record_ids.get(name))
        .collect();

    for link in &db_links {
        if link.kind != data::BridgeKind::P2p {
            continue;
        }

        // Only process links that involve one of the cold-booted nodes
        let involves_target = target_record_ids
            .iter()
            .any(|id| link.node_a == **id || link.node_b == **id);
        if !involves_target {
            continue;
        }

        let ifindex_a = network::get_ifindex(&link.tap_a)
            .await
            .context(format!("failed to get ifindex for {}", link.tap_a))?;
        let ifindex_b = network::get_ifindex(&link.tap_b)
            .await
            .context(format!("failed to get ifindex for {}", link.tap_b))?;

        network::attach_p2p_redirect(&link.tap_a, ifindex_b)
            .context(format!("failed to attach eBPF redirect on {}", link.tap_a))?;
        network::attach_p2p_redirect(&link.tap_b, ifindex_a)
            .context(format!("failed to attach eBPF redirect on {}", link.tap_b))?;

        // Re-apply link impairment if configured
        if link.delay_us > 0 || link.loss_percent > 0.0 {
            let netem = network::LinkImpairment {
                delay_us: link.delay_us,
                jitter_us: link.jitter_us,
                loss_percent: link.loss_percent,
                reorder_percent: 0.0,
                corrupt_percent: 0.0,
            };
            network::apply_netem(ifindex_a as i32, &netem).await?;
            network::apply_netem(ifindex_b as i32, &netem).await?;
        }

        tracing::info!(
            lab_id = %lab_id,
            tap_a = %link.tap_a,
            tap_b = %link.tap_b,
            "Re-attached eBPF P2p redirect after cold boot"
        );
    }

    Ok(())
}
