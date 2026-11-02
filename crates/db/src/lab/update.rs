use anyhow::{Context, Result, anyhow};
use shared::data::{DbLab, LabState, RecordId};
use tracing::instrument;

use crate::Database;
use crate::lab::create::UPDATE_LAB;
use crate::lab::validate_lab_id;
use crate::persistence::{LAB_TABLE, enum_text, key_of, lab_columns, lab_from_row, lab_values};
use crate::sql::query_optional;

/// Update an existing lab in the database
///
/// **IMPORTANT:** The `user` field (owner) is immutable and cannot be changed.
/// If the provided lab has a different user than the existing lab, the update will fail.
///
/// # Arguments
/// * `db` - Database connection
/// * `lab` - DbLab with all fields populated (id field is required)
///
/// # Returns
/// The updated DbLab record
///
/// # Errors
/// - If lab.id is None (id is required for updates)
/// - If lab_id validation fails
/// - If lab doesn't exist
/// - If trying to change the owner (user field is immutable)
/// - If unique constraints are violated (lab_id, name+user)
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn update_lab(db: &Database, lab: DbLab) -> Result<DbLab> {
    // Require id field for updates
    let id = lab
        .id
        .as_ref()
        .ok_or_else(|| anyhow!("Cannot update lab without id field"))?;

    // Validate lab_id format
    validate_lab_id(&lab.lab_id)?;

    let key = key_of(id, LAB_TABLE)?;

    // Hold the write lock across the check and the update, so the owner cannot change between.
    let writer = db.writer().await?;

    // Verify the lab exists and check if user is being changed
    let existing = query_optional(
        &writer.conn,
        concat!("SELECT ", lab_columns!(), " FROM lab WHERE id = ?1"),
        [key.as_str()],
        lab_from_row,
    )
    .await
    .context(format!("Failed to fetch existing lab: {:?}", id))?
    .ok_or_else(|| anyhow!("Lab not found: {:?}", id))?;

    // Verify owner (user) is not being changed - it's immutable
    if existing.user != lab.user {
        return Err(anyhow!(
            "Cannot change lab owner: owner is immutable. Existing owner: {:?}, attempted new owner: {:?}",
            existing.user,
            lab.user
        ));
    }

    // Perform update
    let updated = query_optional(
        &writer.conn,
        UPDATE_LAB,
        lab_values(&lab, &key)?,
        lab_from_row,
    )
    .await
    .context(format!("Failed to update lab: {}", lab.name))?;

    updated.ok_or_else(|| anyhow!("Lab update failed: {}", lab.name))
}

/// Update the status field of an existing lab
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - Record ID of the lab to update
/// * `state` - New lab state
///
/// # Returns
/// The updated DbLab record
#[instrument(skip(db), level = "debug")]
pub async fn update_lab_state(db: &Database, lab_id: RecordId, state: LabState) -> Result<DbLab> {
    let writer = db.writer().await?;
    let updated = query_optional(
        &writer.conn,
        concat!(
            "UPDATE lab SET status = ?2 WHERE id = ?1 RETURNING ",
            lab_columns!()
        ),
        [key_of(&lab_id, LAB_TABLE)?, enum_text(&state, "status")?],
        lab_from_row,
    )
    .await
    .context(format!("Failed to update state for lab: {:?}", lab_id))?;

    updated.ok_or_else(|| anyhow!("Lab not found: {:?}", lab_id))
}
