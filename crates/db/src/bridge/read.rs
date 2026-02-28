use anyhow::{Context, Result};
use shared::data::DbBridge;
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;
use surrealdb_types::RecordId;

/// Get a bridge by its ID
///
/// # Arguments
/// * `db` - Database connection
/// * `bridge_id` - RecordId of the bridge
///
/// # Returns
/// The DbBridge record if found
///
/// # Errors
/// - If the bridge doesn't exist
/// - If there's a database error
pub async fn get_bridge(db: &Arc<Surreal<Client>>, bridge_id: &RecordId) -> Result<DbBridge> {
    let bridge: Option<DbBridge> = db
        .select::<Option<DbBridge>>(bridge_id)
        .await
        .context(format!("Failed to get bridge: id={:?}", bridge_id))?;

    bridge.ok_or_else(|| anyhow::anyhow!("Bridge not found: id={:?}", bridge_id))
}

/// Get a bridge by its index and lab
///
/// # Arguments
/// * `db` - Database connection
/// * `index` - Bridge index
/// * `lab_id` - RecordId of the lab
///
/// # Returns
/// The DbBridge record if found
///
/// # Errors
/// - If the bridge doesn't exist
/// - If there's a database error
pub async fn get_bridge_by_index(
    db: &Arc<Surreal<Client>>,
    index: u16,
    lab_id: &RecordId,
) -> Result<DbBridge> {
    let mut result = db
        .query("SELECT * FROM bridge WHERE index = $index AND lab = $lab_id")
        .bind(("index", index))
        .bind(("lab_id", lab_id.clone()))
        .await
        .context(format!(
            "Failed to get bridge by index: index={}, lab_id={:?}",
            index, lab_id
        ))?;

    let bridge: Option<DbBridge> = result.take(0).context("Failed to deserialize bridge")?;

    bridge.ok_or_else(|| anyhow::anyhow!("Bridge not found: index={}, lab_id={:?}", index, lab_id))
}

/// List all bridges for a lab
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - RecordId of the lab
///
/// # Returns
/// Vector of DbBridge records
///
/// # Errors
/// - If there's a database error
pub async fn list_bridges(db: &Arc<Surreal<Client>>, lab_id: &RecordId) -> Result<Vec<DbBridge>> {
    let mut result = db
        .query("SELECT * FROM bridge WHERE lab = $lab_id ORDER BY index")
        .bind(("lab_id", lab_id.clone()))
        .await
        .context(format!(
            "Failed to list bridges for lab: lab_id={:?}",
            lab_id
        ))?;

    let bridges: Vec<DbBridge> = result.take(0).context("Failed to deserialize bridges")?;

    Ok(bridges)
}
