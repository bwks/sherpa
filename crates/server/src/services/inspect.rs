use anyhow::{Context, Result, anyhow};
use regex::Regex;
use shared::data::{
    BridgeInfo, DbNode, DeviceInfo, InspectRequest, InspectResponse, LabInfo, LinkInfo, NodeConfig,
    RecordId,
};
use shared::konst::{LAB_FILE_NAME, SHERPA_LABS_PATH, SHERPA_STORAGE_POOL};
use shared::util::load_file;
use std::str::FromStr;
use std::time::Instant;
use tracing::Instrument;
use uuid::Uuid;
use virt::storage_pool::StoragePool;

use crate::daemon::state::AppState;

/// Inspect a lab and return its current state
///
/// This function queries:
/// - Database for lab metadata and nodes
/// - libvirt for VM domains and storage
/// - Docker for containers (future)
///
/// Authentication is handled by the caller (RPC middleware or web cookie auth)
/// before this function is invoked.
pub async fn inspect_lab(request: InspectRequest, state: &AppState) -> Result<InspectResponse> {
    let req_id = Uuid::now_v7();
    let span = tracing::debug_span!("inspect_lab", %req_id, lab_id = %request.lab_id);

    inspect_lab_inner(request, state).instrument(span).await
}

async fn inspect_lab_inner(request: InspectRequest, state: &AppState) -> Result<InspectResponse> {
    let lab_id = &request.lab_id;
    let username = &request.username;
    let t0 = Instant::now();

    // Get user from database to validate existence and get RecordId
    let db_user = db::get_user(&state.db, username)
        .await
        .context(format!("User '{}' not found in database", username))?;

    let user_id = db_user
        .id
        .ok_or_else(|| anyhow!("User '{}' missing record ID", username))?;

    // Get lab from database
    let db_lab = db::get_lab(&state.db, lab_id)
        .await
        .context(format!("Lab '{}' not found in database", lab_id))?;

    // Validate ownership
    if db_lab.user != user_id {
        return Err(anyhow!(
            "Permission denied: Lab '{}' is owned by another user",
            lab_id
        ));
    }

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

    // List all domains
    let domains = qemu_conn
        .list_all_domains(0)
        .context("Failed to list libvirt domains")?;

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
        "inspect_lab: libvirt connect+domains+pool+volumes took {:?}",
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
        let node_name = node.name.clone();
        let device_name = format!("{}-{}", node_name, lab_id);

        // Look up node image from pre-fetched list
        let node_image = find_node_image(&node_images, &node.image)
            .ok_or_else(|| anyhow!("Node image not found for node '{}'", node_name))?;

        let mut device_info = DeviceInfo {
            name: node_name.clone(),
            model: node_image.model,
            kind: node_image.kind.clone(),
            state: node.state,
            mgmt_ipv4: node.mgmt_ipv4.clone().unwrap_or_default(),
            mgmt_ipv6: None,
            vnc_port: None,
            disks: Vec::new(),
        };

        // Check if device exists in libvirt
        let domain_found = domains
            .iter()
            .find(|d| d.get_name().unwrap_or_default() == device_name);

        if let Some(domain) = domain_found {
            // Extract VNC port from domain XML
            device_info.vnc_port = extract_vnc_port(domain);

            // Filter pre-fetched volumes for this device
            for volume in &volumes {
                if volume.contains(&device_name) {
                    device_info.disks.push(volume.clone());
                }
            }
        }

        // Always add device info, regardless of whether it's active or not
        devices.push(device_info);
    }

    Ok(InspectResponse {
        lab_info,
        devices,
        inactive_devices: Vec::new(), // Keep field for API compatibility
        links,
        bridges,
    })
}

/// Extract the VNC port from a libvirt domain's XML definition.
fn extract_vnc_port(domain: &virt::domain::Domain) -> Option<i32> {
    static RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(
            r#"<graphics[^>]*type=['"]\s*vnc\s*['"][^>]*port=['"]\s*(\d+)\s*['"]([\s\S]*?)/>"#,
        )
        .unwrap_or_else(|e| panic!("invalid VNC port regex: {e}"))
    });

    let xml = domain.get_xml_desc(0).ok()?;
    let caps = RE.captures(&xml)?;
    let port: i32 = caps.get(1)?.as_str().parse().ok()?;
    if port > 0 { Some(port) } else { None }
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
