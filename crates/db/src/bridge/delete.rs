use std::sync::Arc;
use anyhow::{Context, Result};
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;
use surrealdb_types::RecordId;

/// Delete a bridge
///
/// This function deletes a bridge record.
///
/// # Arguments
/// * `db` - Database connection
/// * `bridge_id` - RecordId of the bridge to delete
///
/// # Returns
/// Ok(()) if successful
///
/// # Errors
/// - If the bridge doesn't exist
/// - If there's a database error
pub async fn delete_bridge(db: &Arc<Surreal<Client>>, bridge_id: &RecordId) -> Result<()> {
    let _: Option<RecordId> = db
        .delete::<Option<RecordId>>(bridge_id)
        .await
        .context(format!("Failed to delete bridge: bridge_id={:?}", bridge_id))?;

    Ok(())
}

/// Delete all bridges for a lab
///
/// This function deletes all bridges for a given lab.
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - RecordId of the lab
///
/// # Returns
/// Ok(()) if successful
///
/// # Errors
/// - If there's a database error
pub async fn delete_lab_bridges(db: &Arc<Surreal<Client>>, lab_id: &RecordId) -> Result<()> {
    db.query("DELETE FROM bridge WHERE lab = $lab_id")
        .bind(("lab_id", lab_id.clone()))
        .await
        .context(format!(
            "Failed to delete bridges for lab: lab_id={:?}",
            lab_id
        ))?;

    Ok(())
}
