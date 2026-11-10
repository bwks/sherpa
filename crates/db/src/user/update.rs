use std::error::Error;
use std::fmt;

use anyhow::{Context, Result, anyhow};
use jiff::Timestamp;
use shared::data::{DbUser, RecordId};
use tracing::instrument;
use turso::Value;

use crate::Database;
use crate::helpers::get_user_id;
use crate::persistence::{USER_TABLE, json_text, key_of, user_from_row};
use crate::queries;
use crate::sql::query_optional;

/// `update_user` was given a user read before another write changed the row, so writing it
/// would undo that change. Read the user again and reapply the edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserChanged {
    pub username: String,
}

impl fmt::Display for UserChanged {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "User '{}' was changed by another request; reload and try again",
            self.username
        )
    }
}

impl Error for UserChanged {}

/// Update an existing user in the database
///
/// Writes the user's name, password hash, admin flag and SSH keys, and sets `updated_at` to now.
/// `created_at` is never changed. The `DbUser` must have a valid `id` field set.
///
/// The write is a compare-and-set: it succeeds only if the row's `updated_at` is still the one in
/// `user`, the value it had when it was read. Otherwise another write landed in between, and it
/// fails with [`UserChanged`] instead of overwriting that write.
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
/// - [`UserChanged`] if the row changed since `user` was read
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
        Value::from(user.updated_at.to_string()),
    ];
    let updated = query_optional(&writer.conn, queries::user::UPDATE, values, user_from_row)
    .await
    .context(format!(
        "Failed to update user:\n id: {:?}\n username: {}\nNote: Username change may fail if it conflicts with another user",
        id, user.username
    ))?;

    if let Some(updated) = updated {
        return Ok(updated);
    }
    let current = query_optional(
        &writer.conn,
        queries::user::FIND_BY_ID,
        [key_of(&id, USER_TABLE)?],
        user_from_row,
    )
    .await
    .context(format!(
        "Failed to re-read user {id:?} after a refused update"
    ))?;
    match current {
        Some(current) => Err(UserChanged {
            username: current.username,
        }
        .into()),
        None => Err(anyhow!(
            "User not found for update:\n id: {:?}\n username: {}\n",
            id,
            user.username
        )),
    }
}

/// Set the password hash of user `id`, and end every session of theirs except `keep`.
///
/// It writes only the hash and `updated_at`, so it cannot undo a concurrent edit of the user's
/// other fields, and it does both writes in one transaction. Pass the caller's own session as
/// `keep` when users change their own password; pass `None` when an admin resets someone else's.
/// Returns the number of sessions ended.
///
/// # Errors
/// - If `id` is not a user id, or the user does not exist
/// - If there's a database error; neither write is kept then
#[instrument(skip(db, password_hash, keep), level = "debug")]
pub async fn change_user_password(
    db: &Database,
    id: RecordId,
    password_hash: &str,
    keep: Option<&str>,
) -> Result<usize> {
    let key = key_of(&id, USER_TABLE)?;
    let mut writer = db.writer().await?;
    let tx = writer
        .conn
        .transaction()
        .await
        .context("Failed to start the password change transaction")?;
    query_optional(
        &tx,
        queries::user::UPDATE_PASSWORD,
        (key.as_str(), password_hash, Timestamp::now().to_string()),
        user_from_row,
    )
    .await
    .context(format!("Failed to set the password of user {id:?}"))?
    .ok_or_else(|| anyhow!("User not found for password change: {id:?}"))?;
    // No session has an empty key, so `None` keeps none.
    let ended = tx
        .execute(
            queries::session::DELETE_BY_USER_EXCEPT,
            (key.as_str(), keep.unwrap_or_default()),
        )
        .await
        .context(format!("Failed to end the sessions of user {id:?}"))?;
    tx.commit()
        .await
        .context("Failed to commit the password change")?;
    usize::try_from(ended).context("Ended session count is out of range")
}
