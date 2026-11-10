use anyhow::{Context, Result};
use shared::data::DbSessionUser;
use tracing::instrument;

use crate::Database;
use crate::persistence::session_user_from_row;
use crate::queries;
use crate::sql::query_optional;

/// The session `id` with its user's current name and admin flag, or `None` when the session
/// has ended: logged out, ended by a password change or user deletion, or swept after expiry.
///
/// An expired session that has not been swept yet is returned; the caller checks `expires_at`.
///
/// # Errors
/// - If there's a database error during the query
#[instrument(skip(db, id), level = "debug")]
pub async fn find_session(db: &Database, id: &str) -> Result<Option<DbSessionUser>> {
    let conn = db.reader().await?;
    query_optional(
        &conn,
        queries::session::FIND_WITH_USER,
        [id],
        session_user_from_row,
    )
    .await
    .context("Failed to look up session")
}
