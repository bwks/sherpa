use anyhow::{Context, Result};
use tracing::instrument;

use super::LoginExpiry;
use crate::Database;
use crate::queries;
use crate::sql::execute;

/// Delete every login count that `expiry` says no longer limits anything, and return how many:
/// those whose lock has ended, and those not locked whose window has passed. A lock that has not
/// ended is never deleted.
///
/// # Errors
/// - If there's a database error during deletion
#[instrument(skip(db), level = "debug")]
pub async fn delete_expired_login_limits(db: &Database, expiry: LoginExpiry) -> Result<usize> {
    let writer = db.writer().await?;
    let deleted = execute(
        &writer.conn,
        queries::login_limit::DELETE_EXPIRED.bind(expiry.values()),
    )
    .await
    .context("Failed to delete expired login counts")?;
    usize::try_from(deleted).context("Deleted login count is out of range")
}
