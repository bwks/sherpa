use anyhow::{Context, Result};
use shared::data::{DbLoginAttempt, RecordId};
use tracing::instrument;

use crate::Database;
use crate::persistence::{USER_TABLE, key_of, login_attempt_from_row};
use crate::queries;
use crate::sql::query_all;

/// The stored login attempts of user `id`, oldest first.
///
/// # Errors
/// - If `id` is not a user id
/// - If there's a database error during the query
#[instrument(skip(db), level = "debug")]
pub async fn list_login_attempts(db: &Database, id: RecordId) -> Result<Vec<DbLoginAttempt>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        queries::login_attempt::LIST_BY_USER.bind([key_of(&id, USER_TABLE)?.into()]),
        login_attempt_from_row,
    )
    .await
    .context(format!("Failed to list the login attempts of user {id:?}"))
}
