use anyhow::{Context, Result, anyhow};
use shared::data::{
    BridgeInfo, DbNode, DeviceInfo, InspectResponse, LabInfo, LinkInfo, NodeConfig, RecordId,
};
use shared::konst::{LAB_FILE_NAME, SHERPA_LABS_PATH, SHERPA_STORAGE_POOL};
use shared::util::load_file;
use std::str::FromStr;
use std::time::Instant;
use tracing::Instrument;
use tracing::instrument;
use uuid::Uuid;
use virt::storage_pool::StoragePool;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_lab_access;
use crate::daemon::state::AppState;

/// Inspect a lab and return its current state
///
/// This function queries:
/// - Database for lab metadata and nodes
/// - libvirt for VM domains and storage
/// - Docker for containers (future)
///
/// The caller must own the lab or be an admin.
#[instrument(skip(ctx, state), fields(username = %ctx.username, %lab_id))]
pub async fn inspect_lab(
    ctx: &AuthContext,
    lab_id: &str,
    state: &AppState,
) -> Result<InspectResponse> {
    let req_id = Uuid::now_v7();
    let span = tracing::debug_span!("inspect_lab", %req_id, %lab_id);

    inspect_lab_inner(ctx, lab_id, state).instrument(span).await
}

async fn inspect_lab_inner(
    ctx: &AuthContext,
    lab_id: &str,
    state: &AppState,
) -> Result<InspectResponse> {
    let t0 = Instant::now();

    let db_lab = require_lab_access(ctx, &state.db, lab_id).await?;

    let tailscale = super::tailscale::inspect(state, &db_lab).await;

    let lab_record_id = db_lab
        .id
        .ok_or_else(|| anyhow::anyhow!("Lab missing record ID"))?;

    tracing::debug!("inspect_lab: user+lab DB lookups took {:?}", t0.elapsed());
    let t1 = Instant::now();

    // Load lab info from filesystem
    let lab_dir = format!("{SHERPA_LABS_PATH}/{lab_id}");
    let lab_file = load_file(&format!("{lab_dir}/{LAB_FILE_NAME}"))
        .context("Unable to load lab file. Is the lab running?")?;
    let lab_info = LabInfo::from_str(&lab_file).context("Failed to parse lab info file")?;

    tracing::debug!("inspect_lab: load lab file took {:?}", t1.elapsed());
    let t2 = Instant::now();

    // Run independent async DB operations concurrently
    let (db_nodes, db_links, db_bridges) = tokio::try_join!(
        async {
            db::list_nodes_by_lab(&state.db, lab_record_id.clone())
                .await
                .context("Failed to list nodes for lab")
        },
        async {
            db::list_links_by_lab(&state.db, lab_record_id.clone())
                .await
                .context("Failed to list links for lab")
        },
        async {
            db::list_bridges(&state.db, &lab_record_id)
                .await
                .context("Failed to list bridges for lab")
        },
    )?;

    tracing::debug!("inspect_lab: concurrent DB queries took {:?}", t2.elapsed());
    let t3 = Instant::now();

    // Batch-fetch node images instead of per-node queries
    let mut image_ids: Vec<RecordId> = db_nodes.iter().map(|n| n.image.clone()).collect();
    image_ids.dedup();

    let node_images = db::list_node_images_by_ids(&state.db, image_ids)
        .await
        .context("Failed to batch fetch node images")?;

    tracing::debug!("inspect_lab: batch image fetch took {:?}", t3.elapsed());
    let t4 = Instant::now();

    // Connect to libvirt
    let qemu_conn = state
        .qemu
        .connect()
        .context("Failed to connect to libvirt")?;

    // Get storage pool
    let pool = StoragePool::lookup_by_name(&qemu_conn, SHERPA_STORAGE_POOL).context(format!(
        "Failed to find storage pool '{}'",
        SHERPA_STORAGE_POOL
    ))?;

    // List volumes once before the loop instead of per-node
    let volumes = pool.list_volumes().context(format!(
        "Failed to list volumes in pool '{}'",
        SHERPA_STORAGE_POOL
    ))?;

    tracing::debug!(
        "inspect_lab: libvirt connect+pool+volumes took {:?}",
        t4.elapsed()
    );
    tracing::debug!("inspect_lab: total elapsed {:?}", t0.elapsed());

    let links: Vec<LinkInfo> = db_links
        .into_iter()
        .map(|link| LinkInfo {
            node_a_name: node_name_from_id(&db_nodes, &link.node_a),
            int_a: link.int_a,
            node_b_name: node_name_from_id(&db_nodes, &link.node_b),
            int_b: link.int_b,
            kind: link.kind.to_string(),
        })
        .collect();

    let bridges: Vec<BridgeInfo> = db_bridges
        .into_iter()
        .map(|bridge| {
            let connected_nodes: Vec<String> = bridge
                .nodes
                .iter()
                .map(|rid| node_name_from_id(&db_nodes, rid))
                .collect();
            BridgeInfo {
                bridge_name: bridge.bridge_name,
                network_name: bridge.network_name,
                connected_nodes,
            }
        })
        .collect();

    // Process each node
    let mut devices = Vec::new();

    for node in &db_nodes {
        // Look up node image from pre-fetched list
        let node_image = find_node_image(&node_images, &node.image)
            .ok_or_else(|| anyhow!("Node image not found for node '{}'", node.name))?;

        devices.push(device_info(node, node_image, &volumes, lab_id));
    }

    Ok(InspectResponse {
        tailscale,
        lab_info,
        lab_state: db_lab.status,
        devices,
        inactive_devices: Vec::new(), // Keep field for API compatibility
        links,
        bridges,
    })
}

/// A node's details: its state, addresses and resources from the database, and its disks
/// from the storage pool.
fn device_info(node: &DbNode, image: &NodeConfig, volumes: &[String], lab_id: &str) -> DeviceInfo {
    let device_name = format!("{}-{}", node.name, lab_id);
    let mut disks = Vec::new();
    for volume in volumes {
        if volume.contains(&device_name) {
            disks.push(volume.clone());
        }
    }
    DeviceInfo {
        name: node.name.clone(),
        model: image.model,
        kind: image.kind.clone(),
        state: node.state,
        mgmt_ipv4: node.mgmt_ipv4.clone().unwrap_or_default(),
        mgmt_ipv6: node.mgmt_ipv6.clone(),
        vnc_port: node.vnc_port,
        disks,
        cpu_count: node.cpu_count,
        memory: node.memory,
    }
}

/// Find a node image by RecordId in the pre-fetched list.
fn find_node_image<'a>(images: &'a [NodeConfig], id: &RecordId) -> Option<&'a NodeConfig> {
    images.iter().find(|img| img.id.as_ref() == Some(id))
}

/// Resolve a node RecordId to its name using the already-fetched node list.
fn node_name_from_id(nodes: &[DbNode], rid: &RecordId) -> String {
    nodes
        .iter()
        .find(|n| n.id.as_ref() == Some(rid))
        .map(|n| n.name.clone())
        .unwrap_or_else(|| format!("{:?}", rid))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::{NodeModel, NodeState};

    fn db_node(name: &str) -> DbNode {
        DbNode {
            id: None,
            name: name.to_string(),
            image: RecordId::new("node_image", "ubuntu"),
            index: 1,
            lab: RecordId::new("lab", "lab1"),
            mgmt_ipv4: Some("172.31.0.21".to_string()),
            mgmt_ipv6: None,
            mgmt_mac: None,
            state: NodeState::Running,
            cpu_count: Some(16),
            memory: Some(32768),
            vnc_port: Some(5903),
            ssh_host_key: None,
        }
    }

    #[test]
    fn device_details_come_from_the_database_and_pool() {
        let image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        let volumes = vec![
            "dev01-a1b2c3d4-hdd.qcow2".to_string(),
            "dev02-a1b2c3d4-hdd.qcow2".to_string(),
        ];
        let device = device_info(&db_node("dev01"), &image, &volumes, "a1b2c3d4");
        assert_eq!(device.name, "dev01");
        assert_eq!(device.mgmt_ipv4, "172.31.0.21");
        assert_eq!(device.vnc_port, Some(5903));
        assert_eq!(device.cpu_count, Some(16));
        assert_eq!(device.memory, Some(32768));
        assert_eq!(device.disks, vec!["dev01-a1b2c3d4-hdd.qcow2"]);
    }
}
