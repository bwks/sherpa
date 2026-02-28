use anyhow::{Context, Result, anyhow};
use shared::data::DbLab;
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

use crate::lab::validate_lab_id;

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
/// # Example
/// ```no_run
/// # use db::{connect, get_lab, update_lab};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let mut lab = get_lab(&db, "lab-0001").await?;
/// lab.name = "Updated Lab Name".to_string();
/// let updated = update_lab(&db, lab).await?;
/// assert_eq!(updated.name, "Updated Lab Name");
/// # Ok(())
/// # }
/// ```
pub async fn update_lab(db: &Arc<Surreal<Client>>, lab: DbLab) -> Result<DbLab> {
    // Require id field for updates
    let id = lab
        .id
        .as_ref()
        .ok_or_else(|| anyhow!("Cannot update lab without id field"))?;

    // Validate lab_id format
    validate_lab_id(&lab.lab_id)?;

    // Verify the lab exists and check if user is being changed
    let existing_lab: Option<DbLab> = db
        .select(id.clone())
        .await
        .context(format!("Failed to fetch existing lab: {:?}", id))?;

    let existing = existing_lab.ok_or_else(|| anyhow!("Lab not found: {:?}", id))?;

    // Verify owner (user) is not being changed - it's immutable
    if existing.user != lab.user {
        return Err(anyhow!(
            "Cannot change lab owner: owner is immutable. Existing owner: {:?}, attempted new owner: {:?}",
            existing.user,
            lab.user
        ));
    }

    // Perform update
    let updated: Option<DbLab> = db
        .update(id.clone())
        .content(lab.clone())
        .await
        .context(format!("Failed to update lab: {}", lab.name))?;

    updated.ok_or_else(|| anyhow!("Lab update failed: {}", lab.name))
}
