use anyhow::{Context, Result, anyhow};
use futures::future::join_all;
use shared::data::{LabNodeActionResponse, NodeActionResult, NodeKind, NodeState, RecordId};

use crate::daemon::state::AppState;

/// Shutdown all (or a specific) node(s) for a lab.
///
/// VMs: if the QEMU guest agent is available, graceful ACPI shutdown via
/// `domain.shutdown()`. Otherwise, forced power-off via `domain.destroy()`.
/// Containers: `docker stop`.
/// DB node state is updated to `Stopped` on success.
pub async fn shutdown_lab_nodes(
    lab_id: &str,
    node_name: Option<&str>,
    state: &AppState,
) -> Result<LabNodeActionResponse> {
    // Get lab from DB to obtain RecordId
    let db_lab = db::get_lab(&state.db, lab_id)
        .await
        .context(format!("Lab '{}' not found in database", lab_id))?;

    let lab_record_id = db_lab.id.ok_or_else(|| anyhow!("Lab missing record ID"))?;

    // Get all nodes for this lab
    let db_nodes = db::list_nodes_by_lab(&state.db, lab_record_id)
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

        futures.push(async move {
            let result = match kind {
                NodeKind::VirtualMachine | NodeKind::Unikernel => {
                    shutdown_vm(&device_name, &node_name, &state).await
                }
                NodeKind::Container => shutdown_container(&device_name, &node_name, &state).await,
            };

            (result, node_name, node_id)
        });
    }

    // Run all operations concurrently
    let concurrent_results = join_all(futures).await;

    // Collect results and update DB state
    let mut results = immediate_results;
    for (result, node_name, node_id) in concurrent_results {
        if result.success
            && let Some(id) = node_id
            && let Err(e) = db::update_node_state(&state.db, id, NodeState::Stopped).await
        {
            tracing::warn!("Failed to update DB state for node '{}': {}", node_name, e);
        }
        results.push(result);
    }

    Ok(LabNodeActionResponse { results })
}

/// Check if the QEMU guest agent is available by sending a ping command.
/// Uses a short timeout (5s) so we don't block long on unresponsive VMs.
fn has_guest_agent(domain: &virt::domain::Domain) -> bool {
    domain
        .qemu_agent_command(r#"{"execute":"guest-ping"}"#, 5, 0)
        .is_ok()
}

async fn shutdown_vm(device_name: &str, node_name: &str, state: &AppState) -> NodeActionResult {
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

        match domain.is_active() {
            Ok(true) => {
                if has_guest_agent(&domain) {
                    // Guest agent available — graceful ACPI shutdown
                    match domain.shutdown() {
                        Ok(_) => Ok(NodeActionResult {
                            name: node_name,
                            success: true,
                            message: "Shutdown initiated (graceful)".to_string(),
                        }),
                        Err(e) => Ok(NodeActionResult {
                            name: node_name,
                            success: false,
                            message: format!("Failed to shutdown: {}", e),
                        }),
                    }
                } else {
                    // No guest agent — force power off
                    match domain.destroy() {
                        Ok(_) => Ok(NodeActionResult {
                            name: node_name,
                            success: true,
                            message: "Powered off (forced, no guest agent)".to_string(),
                        }),
                        Err(e) => Ok(NodeActionResult {
                            name: node_name,
                            success: false,
                            message: format!("Failed to power off: {}", e),
                        }),
                    }
                }
            }
            Ok(false) => Ok(NodeActionResult {
                name: node_name,
                success: true,
                message: "Already stopped".to_string(),
            }),
            Err(e) => Ok(NodeActionResult {
                name: node_name,
                success: false,
                message: format!("Failed to check state: {}", e),
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

async fn shutdown_container(
    device_name: &str,
    node_name: &str,
    state: &AppState,
) -> NodeActionResult {
    match container::pause_container(&state.docker, device_name).await {
        Ok(()) => NodeActionResult {
            name: node_name.to_string(),
            success: true,
            message: "Paused".to_string(),
        },
        Err(e) => NodeActionResult {
            name: node_name.to_string(),
            success: false,
            message: format!("Failed to pause: {:#}", e),
        },
    }
}
