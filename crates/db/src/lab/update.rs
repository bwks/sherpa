use anyhow::{Context, Result};
use shared::data::{DbLab, LabPhase, RecordId};
use tracing::instrument;
use turso::Value;

use crate::Database;
use crate::persistence::{LAB_TABLE, enum_text, key_of, lab_from_row};
use crate::queries;
use crate::sql::query_optional;

/// Record the Tailscale gateway container of the lab `id`, or `None` once it is removed. Only
/// that column is written, so a phase written meanwhile is kept.
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
        queries::lab::UPDATE_TAILSCALE_CONTAINER.bind([
            Value::from(key_of(id, LAB_TABLE)?),
            container_id.map_or(Value::Null, Value::from),
        ]),
        lab_from_row,
    )
    .await
    .with_context(|| format!("Failed to record the Tailscale container of lab {id:?}"))?;
    updated.with_context(|| format!("Lab not found: {id:?}"))
}

/// Move the lab `lab_id` from phase `from` to `to`: a compare-and-set, so a writer that read
/// the lab before another moved it does not overwrite the move.
///
/// Returns the moved lab, or `None` when the lab is no longer in `from` or does not exist.
///
/// # Errors
///
/// Fails when the database cannot be written.
#[instrument(skip(db), level = "debug")]
pub async fn update_lab_phase_from(
    db: &Database,
    lab_id: &str,
    from: LabPhase,
    to: LabPhase,
) -> Result<Option<DbLab>> {
    let writer = db.writer().await?;
    query_optional(
        &writer.conn,
        queries::lab::UPDATE_PHASE_FROM.bind([
            Value::from(lab_id),
            enum_text(&from, "phase")?.into(),
            enum_text(&to, "phase")?.into(),
        ]),
        lab_from_row,
    )
    .await
    .with_context(|| format!("Failed to move lab {lab_id} from {from} to {to}"))
}
