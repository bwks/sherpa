use anyhow::{Context, Result};
use tracing::instrument;
use turso::Connection;

use crate::Database;
use crate::queries;
use crate::sql::query_count;

/// How many failure rows the login limit holds.
///
/// # Errors
/// - If there's a database error during the query
#[instrument(skip(db), level = "debug")]
pub async fn count_login_failures(db: &Database) -> Result<usize> {
    let conn = db.reader().await?;
    rows(&conn).await
}

/// How many rows the failed login table holds.
pub(super) async fn rows(conn: &Connection) -> Result<usize> {
    query_count(conn, queries::login_failure::COUNT.bind([]))
        .await
        .context("Failed to count failed logins")
}
