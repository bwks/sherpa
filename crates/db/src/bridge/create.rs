use anyhow::{Context, Result};
use data::{DbBridge, RecordId};
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

/// Create a new shared bridge for a lab
///
/// This function creates a bridge record that represents a shared network
/// segment connecting multiple nodes.
///
/// # Arguments
/// * `db` - Database connection
/// * `index` - Bridge index (0-65535, unique per lab)
/// * `bridge_name` - Linux bridge name on the host
/// * `network_name` - Libvirt network name
/// * `lab_id` - RecordId of the lab this bridge belongs to
/// * `nodes` - Vector of node RecordIds connected to this bridge
///
/// # Returns
/// The created DbBridge record with generated ID
///
/// # Errors
/// - If unique constraint is violated (index, lab combination)
/// - If lab doesn't exist
/// - If there's a database error
pub async fn create_bridge(
    db: &Surreal<Client>,
    index: u16,
    bridge_name: String,
    network_name: String,
    lab_id: RecordId,
    nodes: Vec<RecordId>,
) -> Result<DbBridge> {
    let bridge: Option<DbBridge> = db
        .create("bridge")
        .content(DbBridge {
            id: None,
            index,
            bridge_name: bridge_name.clone(),
            network_name: network_name.clone(),
            lab: lab_id.clone(),
            nodes,
        })
        .await
        .context(format!(
            "Failed to create bridge: index={}, bridge_name={}, lab_id={}",
            index, bridge_name, lab_id
        ))?;

    bridge.ok_or_else(|| {
        anyhow::anyhow!(
            "Bridge was not created: index={}, bridge_name={}, lab_id={}",
            index,
            bridge_name,
            lab_id
        )
    })
}
