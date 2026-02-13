use anyhow::{Context, Result};
use shared::data::{DeviceInfo, InspectResponse, LabInfo, NodeKind, NodeModel};
use shared::konst::{LAB_FILE_NAME, SHERPA_BASE_DIR, SHERPA_LABS_DIR, SHERPA_STORAGE_POOL};
use shared::util::{get_dhcp_leases, load_file};
use std::str::FromStr;
use virt::storage_pool::StoragePool;

use crate::daemon::state::AppState;

/// Inspect a lab and return its current state
///
/// This function queries:
/// - Database for lab metadata and nodes
/// - libvirt for VM domains and storage
/// - Docker for containers (future)
/// - DHCP for management IPs
pub async fn inspect_lab(lab_id: &str, state: &AppState) -> Result<InspectResponse> {
    // Load lab info from filesystem
    let lab_dir = format!("{SHERPA_BASE_DIR}/{SHERPA_LABS_DIR}/{lab_id}");
    let lab_file = load_file(&format!("{lab_dir}/{LAB_FILE_NAME}"))
        .context("Unable to load lab file. Is the lab running?")?;
    let lab_info = LabInfo::from_str(&lab_file)
        .context("Failed to parse lab info file")?;

    // Get lab from database
    let db_lab = db::get_lab(&state.db, lab_id)
        .await
        .context(format!("Lab '{}' not found in database", lab_id))?;

    let lab_record_id = db_lab.id.ok_or_else(|| anyhow::anyhow!("Lab missing record ID"))?;

    // Get nodes from database
    let db_nodes = db::list_nodes_by_lab(&state.db, lab_record_id.clone())
        .await
        .context("Failed to list nodes for lab")?;

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
    let pool = StoragePool::lookup_by_name(&qemu_conn, SHERPA_STORAGE_POOL)
        .context(format!("Failed to find storage pool '{}'", SHERPA_STORAGE_POOL))?;

    // Get DHCP leases for management IPs
    let leases = get_dhcp_leases(&state.config)
        .await
        .context("Failed to get DHCP leases")?;

    // Process each node
    let mut devices = Vec::new();
    let mut inactive_devices = Vec::new();

    for node in db_nodes {
        let node_name = node.name.clone();
        let device_name = format!("{}-{}", node_name, lab_id);

        // Get node config to determine model and kind
        let node_config = db::get_node_config_by_id(&state.db, node.config.clone())
            .await
            .context(format!("Failed to get config for node '{}'", node_name))?
            .context(format!("Node config not found for node '{}'", node_name))?;

        let mut device_info = DeviceInfo {
            name: node_name.clone(),
            model: node_config.model.clone(),
            kind: node_config.kind.clone(),
            active: false,
            mgmt_ip: String::new(),
            disks: Vec::new(),
        };

        // Check if device is running
        let domain_found = domains
            .iter()
            .find(|d| d.get_name().unwrap_or_default() == device_name);

        if let Some(domain) = domain_found {
            // Check if domain is active
            device_info.active = domain
                .is_active()
                .context(format!("Failed to check if domain '{}' is active", device_name))?;

            // Get management IP from DHCP leases
            if let Some(lease) = leases.iter().find(|l| l.hostname == node_name) {
                device_info.mgmt_ip = lease.ipv4_address.clone();
            }

            // Get disk volumes for this device
            let volumes = pool
                .list_volumes()
                .context(format!("Failed to list volumes in pool '{}'", SHERPA_STORAGE_POOL))?;

            for volume in volumes {
                if volume.contains(&device_name) {
                    device_info.disks.push(volume);
                }
            }

            devices.push(device_info);
        } else {
            // Device not found in libvirt, mark as inactive
            inactive_devices.push(node_name);
        }
    }

    Ok(InspectResponse {
        lab_name: db_lab.name,
        lab_id: lab_id.to_string(),
        lab_info,
        devices,
        inactive_devices,
    })
}
