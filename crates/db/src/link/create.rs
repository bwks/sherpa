use anyhow::{Context, Result};
use shared::data::DbLink;
use tracing::instrument;

use crate::Database;
use crate::persistence::{link_from_row, link_values, new_key};
use crate::queries;
use crate::sql::query_optional;

/// Create `link`, a network link between two nodes of a lab, with its impairments, in one write.
/// Its `id` is ignored: the new link gets a generated one.
///
/// # Errors
///
/// Fails when a node at either end does not exist, an interface is already in use, or the
/// database cannot be written.
#[instrument(skip(db), fields(index = link.index), level = "debug")]
pub async fn create_link(db: &Database, link: DbLink) -> Result<DbLink> {
    let described = || {
        format!(
            "index={}, node_a={:?}, node_b={:?}, int_a={}, int_b={}",
            link.index, link.node_a, link.node_b, link.int_a, link.int_b
        )
    };
    let writer = db.writer().await?;
    let created = query_optional(
        &writer.conn,
        queries::link::CREATE,
        link_values(&link, &new_key())?,
        link_from_row,
    )
    .await
    .with_context(|| format!("Failed to create link: {}", described()))?;
    created.with_context(|| format!("Link was not created: {}", described()))
}
