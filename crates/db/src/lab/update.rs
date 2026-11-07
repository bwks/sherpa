use anyhow::{Context, Result, anyhow};
use shared::data::{DbLab, LabState, RecordId};
use tracing::instrument;
use turso::Value;

use crate::Database;
use crate::persistence::{LAB_TABLE, enum_text, key_of, lab_from_row};
use crate::queries;
use crate::sql::query_optional;

/// Record the Tailscale gateway container of the lab `id`, or `None` once it is removed. Only
/// that column is written, so a status written meanwhile is kept.
///
/// # Errors
///
/// Fails when the lab does not exist or the database cannot be written.
#[instrument(skip(db), level = "debug")]
pub async fn update_lab_tailscale_container(
    db: &Database,
    id: &RecordId,
    container_id: Option<&str>,
) -> Result<DbLab> {
    let writer = db.writer().await?;
    let updated = query_optional(
        &writer.conn,
        queries::lab::UPDATE_TAILSCALE_CONTAINER,
        vec![
            Value::from(key_of(id, LAB_TABLE)?),
            container_id.map_or(Value::Null, Value::from),
        ],
        lab_from_row,
    )
    .await
    .with_context(|| format!("Failed to record the Tailscale container of lab {id:?}"))?;
    updated.with_context(|| format!("Lab not found: {id:?}"))
}

/// Update the status field of an existing lab
///
/// # Arguments
/// * `db` - Database connection
/// * `lab` - Record ID of the lab to update
/// * `state` - New lab state
///
/// # Returns
/// The updated `DbLab` record
///
/// # Errors
///
/// Fails when the lab does not exist or the database cannot be written.
#[instrument(skip(db), level = "debug")]
pub async fn update_lab_state(db: &Database, lab: RecordId, state: LabState) -> Result<DbLab> {
    let writer = db.writer().await?;
    let updated = query_optional(
        &writer.conn,
        queries::lab::UPDATE_STATUS,
        [key_of(&lab, LAB_TABLE)?, enum_text(&state, "status")?],
        lab_from_row,
    )
    .await
    .context(format!("Failed to update state for lab: {lab:?}"))?;

    updated.ok_or_else(|| anyhow!("Lab not found: {lab:?}"))
}
