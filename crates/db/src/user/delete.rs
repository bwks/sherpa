use anyhow::{Context, Result, anyhow};
use shared::data::{DbUser, RecordId};
use shared::util::normalize_username;
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;
use tracing::instrument;

use crate::helpers::get_user_id;
use crate::persistence::{UserRow, to_surreal_id};

/// Delete a user by RecordId
///
/// The schema rejects deleting a user who still owns labs (`REFERENCE ON DELETE REJECT`),
/// so their labs must be destroyed first. Callers that want a clear message should check
/// for owned labs before calling this.
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - RecordId of the user to delete
///
/// # Returns
/// `Ok(())` on successful deletion
///
/// # Errors
/// - If the user owns labs
/// - If the record doesn't exist
/// - If there's a database error during deletion
///
#[instrument(skip(db), level = "debug")]
pub async fn delete_user(db: &Arc<Surreal<Client>>, id: RecordId) -> Result<()> {
    // Execute DELETE query
    let deleted: Option<UserRow> = db
        .delete(to_surreal_id(&id))
        .await
        .context(format!("Failed to delete user: {:?}", id))?;

    // Verify the record was found and deleted
    deleted.ok_or_else(|| anyhow!("User not found for deletion: {:?}", id))?;

    Ok(())
}

/// Delete a user by username (convenience function)
///
/// The schema rejects deleting a user who still owns labs (`REFERENCE ON DELETE REJECT`),
/// so their labs must be destroyed first. Callers that want a clear message should check
/// for owned labs before calling this.
///
/// # Arguments
/// * `db` - Database connection
/// * `username` - Username of the user to delete
///
/// # Returns
/// `Ok(())` on successful deletion
///
/// # Errors
/// - If the user is not found
/// - If there's a database error during deletion
///
#[instrument(skip(db), level = "debug")]
pub async fn delete_user_by_username(db: &Arc<Surreal<Client>>, username: &str) -> Result<()> {
    // First get the user to obtain their ID
    let mut response = db
        .query("SELECT * FROM ONLY user WHERE username = $username")
        .bind(("username", normalize_username(username)))
        .await
        .context(format!("Failed to query user: {}", username))?;

    let user: Option<UserRow> = response.take(0)?;
    let user = user
        .map(DbUser::try_from)
        .transpose()?
        .ok_or_else(|| anyhow!("User not found: {}", username))?;

    let id = get_user_id(&user)?;

    // Delete using the ID
    delete_user(db, id).await
}
