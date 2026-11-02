use anyhow::{Context, Result, anyhow};
use shared::data::DbUser;
use tracing::instrument;

use crate::Database;
use crate::helpers::get_user_id;
use crate::persistence::{USER_TABLE, key_of, user_columns, user_from_row, user_values};
use crate::sql::query_optional;

/// Update an existing user in the database
///
/// This performs a full replacement of all fields except the id.
/// The DbUser must have a valid `id` field set.
///
/// **Note:** Changing the username may fail if it conflicts with another user's
/// username due to the unique constraint.
///
/// # Arguments
/// * `db` - Database connection
/// * `user` - DbUser with updated fields and a valid `id`
///
/// # Returns
/// The updated DbUser on success
///
/// # Errors
/// - If the user has no ID
/// - If the record doesn't exist in the database
/// - If the new username conflicts with an existing user
/// - If the update would delete, rename or demote the built-in admin account
/// - If there's a database error during the update
///
#[instrument(skip(db, user), level = "debug")]
pub async fn update_user(db: &Database, user: DbUser) -> Result<DbUser> {
    // Extract and validate the ID
    let id = get_user_id(&user)?;
    let key = key_of(&id, USER_TABLE)?;

    let writer = db.writer().await?;
    let updated = query_optional(
        &writer.conn,
        concat!(
            "UPDATE user SET username = ?2, password_hash = ?3, is_admin = ?4, ssh_keys = ?5, \
             created_at = ?6, updated_at = ?7 WHERE id = ?1 RETURNING ",
            user_columns!()
        ),
        user_values(&user, &key)?,
        user_from_row,
    )
    .await
    .context(format!(
        "Failed to update user:\n id: {:?}\n username: {}\nNote: Username change may fail if it conflicts with another user",
        id, user.username
    ))?;

    // Return result or error if not found
    updated.ok_or_else(|| {
        anyhow!(
            "User not found for update:\n id: {:?}\n username: {}\n",
            id,
            user.username
        )
    })
}
