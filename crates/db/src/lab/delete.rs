use anyhow::{Context, Result};
use shared::data::LabPhase;
use tracing::instrument;
use turso::{Row, Value};

use crate::Database;
use crate::persistence::{Cells, enum_text};
use crate::queries;
use crate::sql::query_optional;

/// Delete the lab `lab_id` if it is still in phase `from`: a compare-and-set, so a lab another
/// writer moved meanwhile is kept.
///
/// Deleting the lab record also deletes its nodes, links, bridges and bridge members: their
/// foreign keys to the lab cascade.
///
/// Returns whether the lab was deleted: `false` when it is in another phase or does not exist.
///
/// # Errors
///
/// Fails when the database cannot be written.
#[instrument(skip(db), level = "debug")]
pub async fn delete_lab_from(db: &Database, lab_id: &str, from: LabPhase) -> Result<bool> {
    let writer = db.writer().await?;
    let deleted = query_optional(
        &writer.conn,
        queries::lab::DELETE_FROM.bind([Value::from(lab_id), enum_text(&from, "phase")?.into()]),
        key_from_row,
    )
    .await
    .with_context(|| format!("Failed to delete lab {lab_id}"))?;
    Ok(deleted.is_some())
}

fn key_from_row(row: &Row) -> Result<String> {
    Cells::new(row).text("id")
}
