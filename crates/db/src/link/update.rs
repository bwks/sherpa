use anyhow::{Context, Result};
use shared::data::{DbLink, LinkImpairment, RecordId};
use tracing::instrument;
use turso::Value;

use crate::Database;
use crate::persistence::{LINK_TABLE, impairment_values, key_of, link_from_row};
use crate::queries;
use crate::sql::query_optional;

/// Set the impairments of the link `id`, in each direction. Only those columns are written, so
/// the rest of the link is kept as stored.
///
/// # Errors
///
/// Fails when the link does not exist or the database cannot be written.
#[instrument(skip(db), level = "debug")]
pub async fn update_link_impairments(
    db: &Database,
    id: &RecordId,
    a_to_b: &LinkImpairment,
    b_to_a: &LinkImpairment,
) -> Result<DbLink> {
    let [a_delay, a_jitter, a_loss, a_reorder, a_corrupt] = impairment_values(a_to_b);
    let [b_delay, b_jitter, b_loss, b_reorder, b_corrupt] = impairment_values(b_to_a);
    let values = [
        Value::from(key_of(id, LINK_TABLE)?),
        a_delay,
        a_jitter,
        a_loss,
        a_reorder,
        a_corrupt,
        b_delay,
        b_jitter,
        b_loss,
        b_reorder,
        b_corrupt,
    ];
    let writer = db.writer().await?;
    let updated = query_optional(
        &writer.conn,
        queries::link::UPDATE_IMPAIRMENTS.bind(values),
        link_from_row,
    )
    .await
    .with_context(|| format!("Failed to update the impairments of link {id:?}"))?;
    updated.with_context(|| format!("Link not found: {id:?}"))
}
