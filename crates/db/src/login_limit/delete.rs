use anyhow::{Context, Result};
use jiff::Timestamp;
use tracing::instrument;

use super::{LoginFailureId, LoginSubject};
use crate::Database;
use crate::queries;
use crate::sql::execute;

/// Delete the failure row `failure` that an attempt of `subject` wrote, and no other, for an
/// attempt that ended without an answer. A row already gone is left so.
///
/// The row is matched on its `rowid`, `subject` and `failed_at` together, so a `rowid` reused
/// after the row was deleted names someone else's row only when that row is the same failure.
///
/// # Errors
/// - If there's a database error during deletion
#[instrument(skip(db, subject), level = "debug")]
pub async fn delete_login_failure(
    db: &Database,
    subject: &LoginSubject,
    failure: LoginFailureId,
) -> Result<()> {
    let writer = db.writer().await?;
    let [rowid, failed_at] = failure.values();
    execute(
        &writer.conn,
        queries::login_failure::DELETE_ONE.bind([
            rowid,
            subject.username.as_str().into(),
            subject.address.value(),
            failed_at,
        ]),
    )
    .await
    .context("Failed to delete the failed login")?;
    Ok(())
}

/// Delete every failure row of `subject`'s username and address together, for a successful
/// login. Rows of the username from other addresses, and of the address for other usernames,
/// stay.
///
/// # Errors
/// - If there's a database error during deletion
#[instrument(skip(db, subject), level = "debug")]
pub async fn clear_login_failures(db: &Database, subject: &LoginSubject) -> Result<()> {
    let writer = db.writer().await?;
    execute(
        &writer.conn,
        queries::login_failure::DELETE_BY_PAIR
            .bind([subject.username.as_str().into(), subject.address.value()]),
    )
    .await
    .context("Failed to clear the failed logins")?;
    Ok(())
}

/// Delete every failure row made at or before `expired_by`, which counts toward no limit any
/// more, and return how many.
///
/// # Errors
/// - If there's a database error during deletion
#[instrument(skip(db), level = "debug")]
pub async fn delete_expired_login_failures(db: &Database, expired_by: Timestamp) -> Result<usize> {
    let writer = db.writer().await?;
    let deleted = execute(
        &writer.conn,
        queries::login_failure::DELETE_EXPIRED.bind([expired_by.as_second().into()]),
    )
    .await
    .context("Failed to delete expired failed logins")?;
    usize::try_from(deleted).context("Deleted failed login count is out of range")
}
