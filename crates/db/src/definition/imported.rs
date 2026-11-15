//! The mark on a node whose definition came from the startup import of a saved manifest.

use anyhow::{Context, Result};
use tracing::instrument;

use shared::data::RecordId;

use crate::Database;
use crate::persistence::{LAB_TABLE, key_of};
use crate::queries::definition;
use crate::sql::{execute, query_count};

/// Mark every node of the lab `lab_id` as imported from a saved manifest. Returns how many nodes
/// were marked.
///
/// # Errors
///
/// Fails when the database cannot be written.
#[instrument(skip(db), fields(%lab_id), level = "debug")]
pub async fn mark_definition_imported(db: &Database, lab_id: &RecordId) -> Result<usize> {
    let lab_key = key_of(lab_id, LAB_TABLE)?;
    let writer = db.writer().await?;
    execute(
        &writer.conn,
        definition::MARK_IMPORTED.bind([lab_key.as_str().into()]),
    )
    .await
    .context("Failed to mark the lab's nodes as imported")
    .and_then(|marked| usize::try_from(marked).context("Too many nodes marked to count"))
}

/// Whether the node `node_name` of the lab `lab_id` is marked as imported from a saved manifest.
///
/// # Errors
///
/// Fails when the database cannot be read.
#[instrument(skip(db), fields(%lab_id, %node_name), level = "debug")]
pub async fn node_definition_imported(
    db: &Database,
    lab_id: &RecordId,
    node_name: &str,
) -> Result<bool> {
    let lab_key = key_of(lab_id, LAB_TABLE)?;
    let conn = db.reader().await?;
    let marked = query_count(
        &conn,
        definition::COUNT_NODE_IMPORTED.bind([lab_key.as_str().into(), node_name.into()]),
    )
    .await
    .context("Failed to read the node's import mark")?;
    Ok(marked > 0)
}
