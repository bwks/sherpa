use anyhow::{Context, Result};
use jiff::Timestamp;
use tracing::instrument;

use crate::Database;
use crate::queries;
use crate::sql::execute;

/// End the session `id`. Returns whether it existed; ending an ended session is not an error.
///
/// # Errors
/// - If there's a database error during deletion
#[instrument(skip(db, id), level = "debug")]
pub async fn delete_session(db: &Database, id: &str) -> Result<bool> {
    let writer = db.writer().await?;
    let deleted = execute(
        &writer.conn,
        queries::session::DELETE_BY_ID.bind([id.into()]),
    )
    .await
    .context("Failed to end session")?;
    Ok(deleted > 0)
}

/// Delete every session whose expiry is at or before `now`, and return how many.
///
/// # Errors
/// - If there's a database error during deletion
#[instrument(skip(db), level = "debug")]
pub async fn delete_expired_sessions(db: &Database, now: Timestamp) -> Result<usize> {
    let writer = db.writer().await?;
    let deleted = execute(
        &writer.conn,
        queries::session::DELETE_EXPIRED.bind([now.as_second().into()]),
    )
    .await
    .context("Failed to delete expired sessions")?;
    usize::try_from(deleted).context("Deleted session count is out of range")
}
