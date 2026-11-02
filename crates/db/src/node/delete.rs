use anyhow::{Context, Result, anyhow};
use shared::data::RecordId;
use tracing::instrument;
use turso::{Connection, Row};

use crate::Database;
use crate::node::read::select_node;
use crate::persistence::{Cells, LAB_TABLE, NODE_TABLE, key_of};
use crate::sql::{query_count, query_optional};

fn deleted_key_from_row(row: &Row) -> Result<String> {
    Cells::new(row).text("id")
}

/// Delete the node `id` on `conn`, failing when it does not exist.
async fn delete_node_row(conn: &Connection, id: &RecordId) -> Result<()> {
    query_optional(
        conn,
        "DELETE FROM node WHERE id = ?1 RETURNING id",
        [key_of(id, NODE_TABLE)?],
        deleted_key_from_row,
    )
    .await
    .context(format!("Failed to delete node: {:?}", id))?
    .ok_or_else(|| anyhow!("Node not found with id: {:?}", id))?;
    Ok(())
}

/// Delete a node by its RecordId (surrogate key)
///
/// Deleting a node also deletes its links and its bridge memberships: their node columns
/// are `ON DELETE CASCADE`. To refuse deletion while links exist, use `delete_node_safe()`.
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The RecordId of the node
///
/// # Errors
/// - If node not found
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn delete_node(db: &Database, id: RecordId) -> Result<()> {
    let writer = db.writer().await?;
    delete_node_row(&writer.conn, &id).await
}

/// Alias for delete_node - kept for API consistency
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The RecordId of the node
///
/// # Errors
/// - If node not found
/// - If there's a database error
#[instrument(skip(db), level = "debug")]
pub async fn delete_node_by_id(db: &Database, id: RecordId) -> Result<()> {
    delete_node(db, id).await
}

/// Delete all nodes for a lab
///
/// The nodes' links and bridge memberships are deleted with them.
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - The RecordId of the lab
///
/// # Errors
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn delete_nodes_by_lab(db: &Database, lab_id: RecordId) -> Result<()> {
    let writer = db.writer().await?;
    writer
        .conn
        .execute(
            "DELETE FROM node WHERE lab = ?1",
            [key_of(&lab_id, LAB_TABLE)?],
        )
        .await
        .context(format!("Failed to delete nodes for lab: {:?}", lab_id))?;

    Ok(())
}

/// Delete all links associated with a node
///
/// Deletes all links where the node appears as either node_a or node_b, leaving the
/// node itself in place.
///
/// # Arguments
/// * `db` - Database connection
/// * `node_id` - The RecordId of the node
///
/// # Errors
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn delete_node_links(db: &Database, node_id: RecordId) -> Result<()> {
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

/// Delete a node and cascade delete all dependent records.
///
/// Deletes the node record. The database deletes its links and bridge memberships through
/// the `ON DELETE CASCADE` foreign keys defined in the schema.
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The RecordId of the node
///
/// # Errors
/// - If node not found
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn delete_node_cascade(db: &Database, id: RecordId) -> Result<()> {
    delete_node(db, id).await
}

/// Delete a node safely (only if it has no links)
///
/// This function checks if the node has any associated links before deletion.
/// If links exist, it returns an error with details about what's blocking deletion.
/// The check and the deletion run under one write lock.
///
/// Use this when you want to prevent accidental deletion of nodes that still have
/// active network connections.
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The RecordId of the node
///
/// # Errors
/// - If node not found
/// - If node has links (won't delete)
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn delete_node_safe(db: &Database, id: RecordId) -> Result<()> {
    let writer = db.writer().await?;

    // Get the node to verify it exists
    let node = select_node(&writer.conn, &id).await?;

    // Check for links (where node appears as either node_a or node_b)
    let links = query_count(
        &writer.conn,
        "SELECT count(*) FROM link WHERE node_a = ?1 OR node_b = ?1",
        [key_of(&id, NODE_TABLE)?],
    )
    .await
    .context("Failed to check for links")?;

    if links > 0 {
        return Err(anyhow!(
            "Cannot delete node '{}' ({:?}): node has {} associated link(s). Delete links first or use delete_node_cascade()",
            node.name,
            id,
            links
        ));
    }

    // Safe to delete - node has no links
    delete_node_row(&writer.conn, &id).await
}
