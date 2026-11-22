use anyhow::Result;
use tracing::instrument;

use super::LoginKeys;
use super::create::sizes;
use crate::Database;

/// How many rows the login limit holds for pairs, addresses and usernames.
///
/// # Errors
/// - If there's a database error during the query
#[instrument(skip(db), level = "debug")]
pub async fn count_login_limits(db: &Database) -> Result<LoginKeys<usize>> {
    let conn = db.reader().await?;
    sizes(&conn).await
}
