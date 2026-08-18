use anyhow::{Context, Result};
use shared::data::{DbBridge, RecordId};
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;
use tracing::instrument;

use crate::persistence::BridgeRow;

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
#[instrument(skip(db), level = "debug")]
pub async fn create_bridge(
    db: &Arc<Surreal<Client>>,
    index: u16,
    bridge_name: String,
    network_name: String,
    lab_id: RecordId,
    nodes: Vec<RecordId>,
) -> Result<DbBridge> {
    let domain = DbBridge {
        id: None,
        index,
        bridge_name: bridge_name.clone(),
        network_name: network_name.clone(),
        lab: lab_id.clone(),
        nodes,
    };
    let bridge: Option<BridgeRow> = db
        .create("bridge")
        .content(BridgeRow::try_from(&domain)?)
        .await
        .context(format!(
            "Failed to create bridge: index={}, bridge_name={}, lab_id={:?}",
            index, bridge_name, lab_id
        ))?;

    bridge.map(DbBridge::try_from).transpose()?.ok_or_else(|| {
        anyhow::anyhow!(
            "Bridge was not created: index={}, bridge_name={}, lab_id={:?}",
            index,
            bridge_name,
            lab_id
        )
    })
}
