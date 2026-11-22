use anyhow::{Context, Result};
use shared::util::normalize_username;
use tracing::instrument;

use crate::Database;
use crate::persistence::Cells;
use crate::queries;
use crate::sql::{query_all, query_optional};

/// Unlock the account `username` and set its failed login count back to 0. Returns whether the
/// user exists; unlocking an account that is not locked is not an error.
///
/// # Errors
/// - If there's a database error during the update
#[instrument(skip(db), level = "debug")]
pub async fn unlock_user(db: &Database, username: &str) -> Result<bool> {
    let writer = db.writer().await?;
    let unlocked = query_optional(
        &writer.conn,
        queries::user::UNLOCK.bind([normalize_username(username).into()]),
        |_| Ok(()),
    )
    .await
    .context(format!("Failed to unlock user {username}"))?;
    Ok(unlocked.is_some())
}

/// The usernames of the locked accounts, in order.
///
/// # Errors
/// - If there's a database error during the query
#[instrument(skip(db), level = "debug")]
pub async fn list_locked_usernames(db: &Database) -> Result<Vec<String>> {
    let conn = db.reader().await?;
    query_all(&conn, queries::user::LIST_LOCKED.bind([]), |row| {
        Cells::new(row).text("username")
    })
    .await
    .context("Failed to list the locked accounts")
}
