use anyhow::{Context, Result, anyhow};
use tracing::instrument;
use turso::Row;

use crate::Database;
use crate::persistence::Cells;
use crate::queries;
use crate::sql::query_optional;

/// Delete a lab by its `lab_id` (business key)
///
/// Deleting the lab record also deletes its nodes, links, bridges and bridge members:
/// their foreign keys to the lab cascade.
///
/// `delete_lab()` does the same.
/// To check for dependencies before deletion, use `delete_lab_safe()`.
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - The unique `lab_id` string
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
        queries::lab::DELETE_BY_LAB_ID,
        [lab_id],
        key_from_row,
    )
    .await
    .context(format!("Failed to delete lab: {lab_id}"))?;

    deleted.ok_or_else(|| anyhow!("Lab with lab_id not found: {lab_id}"))?;
    Ok(())
}

fn key_from_row(row: &Row) -> Result<String> {
    Cells::new(row).text("id")
}
