use anyhow::{Context, Result};
use shared::data::{DbLink, RecordId};
use tracing::instrument;

use crate::Database;
use crate::persistence::{LAB_TABLE, key_of, link_from_row};
use crate::queries;
use crate::sql::query_all;

/// List all links in a specific lab
///
/// # Arguments
/// * `db` - Database connection
/// * `lab` - Lab's `RecordId`
///
/// # Returns
/// Vector of `DbLink` records in the lab
///
/// # Errors
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn list_links_by_lab(db: &Database, lab: RecordId) -> Result<Vec<DbLink>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        queries::link::LIST_BY_LAB,
        [key_of(&lab, LAB_TABLE)?],
        link_from_row,
    )
    .await
    .context(format!("Failed to list links for lab: {lab:?}"))
}
