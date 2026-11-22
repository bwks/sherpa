use anyhow::{Context, Result};
use tracing::instrument;

use super::LoginExpiry;
use crate::Database;
use crate::queries;
use crate::sql::execute;

/// Delete every login count that `expiry` says no longer limits anything, in the pair, address
/// and username tables, and return how many: those whose lock has ended, and those not locked
/// whose window has passed. A lock that has not ended is never deleted.
///
/// # Errors
/// - If there's a database error during deletion; tables swept before it stay swept
#[instrument(skip(db), level = "debug")]
pub async fn delete_expired_login_limits(db: &Database, expiry: LoginExpiry) -> Result<usize> {
    let writer = db.writer().await?;
    let mut deleted = 0;
    for query in [
        queries::login_limit_pair::DELETE_EXPIRED,
        queries::login_limit_address::DELETE_EXPIRED,
        queries::login_limit_username::DELETE_EXPIRED,
    ] {
        deleted += execute(&writer.conn, query.bind(expiry.values()))
            .await
            .context("Failed to delete expired login counts")?;
    }
    usize::try_from(deleted).context("Deleted login count is out of range")
}
