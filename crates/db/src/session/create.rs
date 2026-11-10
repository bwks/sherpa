use anyhow::{Context, Result, anyhow};
use jiff::Timestamp;
use shared::data::{DbSession, RecordId, SessionChannel};
use tracing::instrument;

use crate::Database;
use crate::persistence::{new_key, session_from_row, session_values};
use crate::queries;
use crate::sql::query_optional;

/// Start a session for `user` that expires at `expires_at`, kept to the second.
///
/// The session gets a new random key, which the caller puts in the token.
///
/// # Errors
/// - If `user` is not a user id, or the user does not exist
/// - If there's a database error during creation
#[instrument(skip(db), level = "debug")]
pub async fn create_session(
    db: &Database,
    user: RecordId,
    channel: SessionChannel,
    expires_at: Timestamp,
) -> Result<DbSession> {
    let expires_at =
        Timestamp::from_second(expires_at.as_second()).context("Session expiry is out of range")?;
    let session = DbSession {
        id: new_key(),
        user,
        channel,
        created_at: Timestamp::now(),
        expires_at,
    };
    let writer = db.writer().await?;
    query_optional(
        &writer.conn,
        queries::session::CREATE,
        session_values(&session)?,
        session_from_row,
    )
    .await
    .context(format!("Failed to start a session for {:?}", session.user))?
    .ok_or_else(|| anyhow!("Session was not created for {:?}", session.user))
}
