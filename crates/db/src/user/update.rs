use anyhow::{Context, Result, anyhow};
use shared::data::DbUser;
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;
use tracing::instrument;

use crate::helpers::get_user_id;

/// Update an existing user in the database
///
/// This performs a full replacement of all fields (username and ssh_keys).
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
/// - If there's a database error during the update
///
#[instrument(skip(db, user), level = "debug")]
#[instrument(skip(db), level = "debug")]
pub async fn update_user(db: &Arc<Surreal<Client>>, user: DbUser) -> Result<DbUser> {
    // Extract and validate the ID
    let id = get_user_id(&user)?;

    // Execute UPDATE query - replaces all fields
    let updated: Option<DbUser> = db
        .update(id.clone())
        .content(user.clone())
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
