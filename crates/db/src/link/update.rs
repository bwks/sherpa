use anyhow::{Context, Result, anyhow};
use shared::data::DbLink;
use tracing::instrument;

use crate::Database;
use crate::persistence::{LINK_TABLE, key_of, link_columns, link_from_row, link_values};
use crate::sql::query_optional;

/// Update an existing link in the database
///
/// **IMPORTANT:** The `lab`, `node_a`, and `node_b` fields are immutable and cannot be changed.
/// If the provided link has different values for these fields than the existing link,
/// the update will fail. Links cannot be moved between labs or re-assigned to different nodes.
///
/// The check and the update run under the database's write lock, so no other write can
/// change the link in between.
///
/// # Arguments
/// * `db` - Database connection
/// * `link` - DbLink with all fields populated (id field is required)
///
/// # Returns
/// The updated DbLink record
///
/// # Errors
/// - If link.id is None (id is required for updates)
/// - If link doesn't exist
/// - If trying to change the lab (lab field is immutable)
/// - If trying to change node_a or node_b (immutable after creation)
/// - If unique constraints are violated (node_a, node_b, int_a, int_b combination)
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn update_link(db: &Database, link: DbLink) -> Result<DbLink> {
    // Require id field for updates
    let id = link
        .id
        .as_ref()
        .ok_or_else(|| anyhow!("Cannot update link without id field"))?;
    let key = key_of(id, LINK_TABLE)?;

    let writer = db.writer().await?;

    // Verify the link exists and check if immutable fields are being changed
    let existing = query_optional(
        &writer.conn,
        concat!("SELECT ", link_columns!(), " FROM link WHERE id = ?1"),
        [key.as_str()],
        link_from_row,
    )
    .await
    .context(format!("Failed to get link by id: {:?}", id))?
    .ok_or_else(|| anyhow!("Link not found with id: {:?}", id))?;

    // Verify lab is not being changed - it's immutable
    if existing.lab != link.lab {
        return Err(anyhow!(
            "Cannot change link lab: lab field is immutable. Links cannot be moved between labs. Existing lab: {:?}, attempted new lab: {:?}",
            existing.lab,
            link.lab
        ));
    }

    // Verify node_a is not being changed - it's immutable
    if existing.node_a != link.node_a {
        return Err(anyhow!(
            "Cannot change link node_a: node_a field is immutable. Existing node_a: {:?}, attempted new node_a: {:?}",
            existing.node_a,
            link.node_a
        ));
    }

    // Verify node_b is not being changed - it's immutable
    if existing.node_b != link.node_b {
        return Err(anyhow!(
            "Cannot change link node_b: node_b field is immutable. Existing node_b: {:?}, attempted new node_b: {:?}",
            existing.node_b,
            link.node_b
        ));
    }

    // Perform update
    let updated = query_optional(
        &writer.conn,
        concat!(
            "UPDATE link SET \"index\" = ?2, kind = ?3, node_a = ?4, node_b = ?5, int_a = ?6, \
             int_b = ?7, lab = ?8, tap_a = ?9, tap_b = ?10, a_to_b_delay_us = ?11, \
             a_to_b_jitter_us = ?12, a_to_b_loss_percent = ?13, a_to_b_reorder_percent = ?14, \
             a_to_b_corrupt_percent = ?15, b_to_a_delay_us = ?16, b_to_a_jitter_us = ?17, \
             b_to_a_loss_percent = ?18, b_to_a_reorder_percent = ?19, \
             b_to_a_corrupt_percent = ?20 WHERE id = ?1 RETURNING ",
            link_columns!()
        ),
        link_values(&link, &key)?,
        link_from_row,
    )
    .await
    .context(format!(
        "Failed to update link: node_a={:?}, node_b={:?}",
        link.node_a, link.node_b
    ))?;

    updated.ok_or_else(|| {
        anyhow!(
            "Link update failed: node_a={:?}, node_b={:?}",
            link.node_a,
            link.node_b
        )
    })
}
