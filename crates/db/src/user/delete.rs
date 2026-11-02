use anyhow::{Context, Result, anyhow};
use shared::data::{DbUser, RecordId};
use shared::util::normalize_username;
use tracing::instrument;

use crate::Database;
use crate::persistence::{USER_TABLE, key_of, user_columns, user_from_row};
use crate::sql::query_optional;

/// Delete a user by RecordId
///
/// A user who still owns labs cannot be deleted (`lab.user` is `ON DELETE RESTRICT`),
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
/// - If the user is the built-in admin account
/// - If the record doesn't exist
/// - If there's a database error during deletion
///
#[instrument(skip(db), level = "debug")]
pub async fn delete_user(db: &Database, id: RecordId) -> Result<()> {
    let writer = db.writer().await?;
    let deleted: Option<DbUser> = query_optional(
        &writer.conn,
        concat!("DELETE FROM user WHERE id = ?1 RETURNING ", user_columns!()),
        [key_of(&id, USER_TABLE)?],
        user_from_row,
    )
    .await
    .context(format!("Failed to delete user: {:?}", id))?;

    // Verify the record was found and deleted
    deleted.ok_or_else(|| anyhow!("User not found for deletion: {:?}", id))?;

    Ok(())
}

/// Delete a user by username (convenience function)
///
/// A user who still owns labs cannot be deleted (`lab.user` is `ON DELETE RESTRICT`),
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
pub async fn delete_user_by_username(db: &Database, username: &str) -> Result<()> {
    let writer = db.writer().await?;
    let deleted: Option<DbUser> = query_optional(
        &writer.conn,
        concat!(
            "DELETE FROM user WHERE username = ?1 RETURNING ",
            user_columns!()
        ),
        [normalize_username(username)],
        user_from_row,
    )
    .await
    .context(format!("Failed to delete user: {}", username))?;

    deleted.ok_or_else(|| anyhow!("User not found: {}", username))?;

    Ok(())
}
