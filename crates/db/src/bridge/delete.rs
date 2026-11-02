use anyhow::{Context, Result};
use shared::data::RecordId;
use tracing::instrument;

use crate::Database;
use crate::persistence::{BRIDGE_TABLE, LAB_TABLE, key_of};

/// Delete a bridge
///
/// This function deletes a bridge record. Its members are deleted with it
/// (`bridge_member.bridge` is `ON DELETE CASCADE`). Deleting a bridge that does not
/// exist is not an error.
///
/// # Arguments
/// * `db` - Database connection
/// * `bridge_id` - RecordId of the bridge to delete
///
/// # Returns
/// Ok(()) if successful
///
/// # Errors
/// - If there's a database error
#[instrument(skip(db), level = "debug")]
pub async fn delete_bridge(db: &Database, bridge_id: &RecordId) -> Result<()> {
    let writer = db.writer().await?;
    writer
        .conn
        .execute(
            "DELETE FROM bridge WHERE id = ?1",
            [key_of(bridge_id, BRIDGE_TABLE)?],
        )
        .await
        .context(format!(
            "Failed to delete bridge: bridge_id={:?}",
            bridge_id
        ))?;

    Ok(())
}

/// Delete all bridges for a lab
///
/// This function deletes all bridges for a given lab, and their members.
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
#[instrument(skip(db), level = "debug")]
pub async fn delete_lab_bridges(db: &Database, lab_id: &RecordId) -> Result<()> {
    let writer = db.writer().await?;
    writer
        .conn
        .execute(
            "DELETE FROM bridge WHERE lab = ?1",
            [key_of(lab_id, LAB_TABLE)?],
        )
        .await
        .context(format!(
            "Failed to delete bridges for lab: lab_id={:?}",
            lab_id
        ))?;

    Ok(())
}
