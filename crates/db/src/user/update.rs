use anyhow::{Context, Result, anyhow};
use jiff::Timestamp;
use shared::data::DbUser;
use tracing::instrument;
use turso::Value;

use crate::Database;
use crate::helpers::get_user_id;
use crate::persistence::{USER_TABLE, json_text, key_of, user_from_row};
use crate::queries;
use crate::sql::query_optional;

/// Update an existing user in the database
///
/// Writes the user's name, password hash, admin flag and SSH keys, and sets `updated_at` to now.
/// `created_at` is never changed. The `DbUser` must have a valid `id` field set.
///
/// **Note:** Changing the username may fail if it conflicts with another user's
/// username due to the unique constraint.
///
/// # Arguments
/// * `db` - Database connection
/// * `user` - `DbUser` with updated fields and a valid `id`
///
/// # Returns
/// The updated `DbUser` on success
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
    let values = vec![
        Value::from(key),
        Value::from(user.username.as_str()),
        Value::from(user.password_hash.as_str()),
        Value::from(user.is_admin),
        Value::from(json_text(&user.ssh_keys, "ssh_keys")?),
        Value::from(Timestamp::now().to_string()),
    ];
    let updated = query_optional(&writer.conn, queries::user::UPDATE, values, user_from_row)
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
