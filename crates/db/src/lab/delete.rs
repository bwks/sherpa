use anyhow::{Context, Result, anyhow};
use shared::data::RecordId;
use tracing::instrument;
use turso::{Connection, Row};

use crate::Database;
use crate::helpers::get_lab_id;
use crate::persistence::{Cells, LAB_TABLE, key_of, lab_columns, lab_from_row};
use crate::sql::{query_count, query_optional};

/// Delete a lab by its lab_id (business key)
///
/// Deleting the lab record also deletes its nodes, links, bridges and bridge members:
/// their foreign keys to the lab cascade.
///
/// `delete_lab_cascade()` does the same.
/// To check for dependencies before deletion, use `delete_lab_safe()`.
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - The unique lab_id string
///
/// # Errors
/// - If lab not found
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn delete_lab(db: &Database, lab_id: &str) -> Result<()> {
    let writer = db.writer().await?;
    let deleted = query_optional(
        &writer.conn,
        "DELETE FROM lab WHERE lab_id = ?1 RETURNING id",
        [lab_id],
        key_from_row,
    )
    .await
    .context(format!("Failed to delete lab: {}", lab_id))?;

    deleted.ok_or_else(|| anyhow!("Lab with lab_id not found: {}", lab_id))?;
    Ok(())
}

/// Delete a lab by its RecordId (surrogate key)
///
/// Deleting the lab record also deletes its nodes, links, bridges and bridge members:
/// their foreign keys to the lab cascade. Deleting a lab that does not exist succeeds.
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The RecordId of the lab
///
/// # Errors
/// - If the id is not a lab id
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn delete_lab_by_id(db: &Database, id: RecordId) -> Result<()> {
    let writer = db.writer().await?;
    writer
        .conn
        .execute("DELETE FROM lab WHERE id = ?1", [key_of(&id, LAB_TABLE)?])
        .await
        .context(format!("Failed to delete lab by id: {:?}", id))?;

    Ok(())
}

/// Delete all nodes for a lab
///
/// Deleting the lab deletes its nodes anyway; this deletes them and keeps the lab.
/// The nodes' links and bridge memberships are deleted with them.
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - The unique lab_id string
///
/// # Errors
/// - If lab not found
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn delete_lab_nodes(db: &Database, lab_id: &str) -> Result<()> {
    let writer = db.writer().await?;
    let key = lab_key(&writer.conn, lab_id).await?;
    writer
        .conn
        .execute("DELETE FROM node WHERE lab = ?1", [key])
        .await
        .context(format!("Failed to delete nodes for lab: {}", lab_id))?;

    Ok(())
}

/// Delete all links for a lab
///
/// Deleting the lab deletes its links anyway; this deletes them and keeps the lab and its nodes.
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - The unique lab_id string
///
/// # Errors
/// - If lab not found
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn delete_lab_links(db: &Database, lab_id: &str) -> Result<()> {
    let writer = db.writer().await?;
    let key = lab_key(&writer.conn, lab_id).await?;
    writer
        .conn
        .execute("DELETE FROM link WHERE lab = ?1", [key])
        .await
        .context(format!("Failed to delete links for lab: {}", lab_id))?;

    Ok(())
}

/// Delete a lab and cascade delete all dependent records.
///
/// Deletes the lab record. The database automatically cascade deletes
/// all associated nodes, links, bridges and bridge members via the `ON DELETE CASCADE`
/// foreign keys defined in the schema.
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - The unique lab_id string
///
/// # Errors
/// - If lab not found
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn delete_lab_cascade(db: &Database, lab_id: &str) -> Result<()> {
    delete_lab(db, lab_id).await
}

/// Delete a lab safely (only if it has no nodes or links)
///
/// This function checks if the lab has any nodes or links before deletion.
/// If dependencies exist, it returns an error with details about what's blocking deletion.
///
/// Use this when you want to prevent accidental deletion of labs that still contain resources.
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - The unique lab_id string
///
/// # Errors
/// - If lab not found
/// - If lab has nodes or links (won't delete)
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn delete_lab_safe(db: &Database, lab_id: &str) -> Result<()> {
    // Hold the write lock across the checks and the delete, so nothing is added between.
    let writer = db.writer().await?;

    // Get the lab to verify it exists
    let lab = query_optional(
        &writer.conn,
        concat!("SELECT ", lab_columns!(), " FROM lab WHERE lab_id = ?1"),
        [lab_id],
        lab_from_row,
    )
    .await
    .context(format!("Failed to query lab from database: {}", lab_id))?
    .ok_or_else(|| anyhow!("Lab with lab_id not found: {}", lab_id))?;
    let key = key_of(&get_lab_id(&lab)?, LAB_TABLE)?;

    // Check for nodes
    let nodes = query_count(
        &writer.conn,
        "SELECT count(*) FROM node WHERE lab = ?1",
        [key.as_str()],
    )
    .await
    .context("Failed to check for nodes")?;

    if nodes > 0 {
        return Err(anyhow!(
            "Cannot delete lab '{}' ({}): lab contains {} node(s). Delete nodes first or use delete_lab_cascade()",
            lab.name,
            lab_id,
            nodes
        ));
    }

    // Check for links
    let links = query_count(
        &writer.conn,
        "SELECT count(*) FROM link WHERE lab = ?1",
        [key.as_str()],
    )
    .await
    .context("Failed to check for links")?;

    if links > 0 {
        return Err(anyhow!(
            "Cannot delete lab '{}' ({}): lab contains {} link(s). Delete links first or use delete_lab_cascade()",
            lab.name,
            lab_id,
            links
        ));
    }

    // Safe to delete - lab is empty
    writer
        .conn
        .execute("DELETE FROM lab WHERE id = ?1", [key])
        .await
        .context(format!("Failed to delete lab: {}", lab_id))?;

    Ok(())
}

/// The stored key of the lab with `lab_id`.
async fn lab_key(conn: &Connection, lab_id: &str) -> Result<String> {
    query_optional(
        conn,
        "SELECT id FROM lab WHERE lab_id = ?1",
        [lab_id],
        key_from_row,
    )
    .await
    .context(format!("Failed to query lab from database: {}", lab_id))?
    .ok_or_else(|| anyhow!("Lab with lab_id not found: {}", lab_id))
}

fn key_from_row(row: &Row) -> Result<String> {
    Cells::new(row).text("id")
}
