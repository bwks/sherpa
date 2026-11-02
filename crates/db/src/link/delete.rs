use anyhow::{Context, Result, anyhow};
use shared::data::RecordId;
use tracing::instrument;

use crate::Database;
use crate::persistence::{LAB_TABLE, LINK_TABLE, NODE_TABLE, key_of};

/// Delete a link by its RecordId (surrogate key)
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The RecordId of the link
///
/// # Errors
/// - If link not found
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn delete_link(db: &Database, id: RecordId) -> Result<()> {
    let writer = db.writer().await?;
    let deleted = writer
        .conn
        .execute("DELETE FROM link WHERE id = ?1", [key_of(&id, LINK_TABLE)?])
        .await
        .context(format!("Failed to delete link: {:?}", id))?;

    if deleted == 0 {
        return Err(anyhow!("Link not found with id: {:?}", id));
    }

    Ok(())
}

/// Alias for delete_link - kept for API consistency
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The RecordId of the link
///
/// # Errors
/// - If link not found
/// - If there's a database error
#[instrument(skip(db), level = "debug")]
pub async fn delete_link_by_id(db: &Database, id: RecordId) -> Result<()> {
    delete_link(db, id).await
}

/// Delete all links for a lab
///
/// This is typically called when deleting a lab to clean up all associated links.
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - The RecordId of the lab
///
/// # Errors
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn delete_links_by_lab(db: &Database, lab_id: RecordId) -> Result<()> {
    let writer = db.writer().await?;
    writer
        .conn
        .execute(
            "DELETE FROM link WHERE lab = ?1",
            [key_of(&lab_id, LAB_TABLE)?],
        )
        .await
        .context(format!("Failed to delete links for lab: {:?}", lab_id))?;

    Ok(())
}

/// Delete all links associated with a node
///
/// This deletes all links where the node appears as either node_a or node_b.
/// Deleting the node itself also deletes its links (`ON DELETE CASCADE`).
///
/// # Arguments
/// * `db` - Database connection
/// * `node_id` - The RecordId of the node
///
/// # Errors
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn delete_links_by_node(db: &Database, node_id: RecordId) -> Result<()> {
    let writer = db.writer().await?;
    writer
        .conn
        .execute(
            "DELETE FROM link WHERE node_a = ?1 OR node_b = ?1",
            [key_of(&node_id, NODE_TABLE)?],
        )
        .await
        .context(format!("Failed to delete links for node: {:?}", node_id))?;

    Ok(())
}
