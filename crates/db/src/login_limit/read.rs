use anyhow::{Context, Result};
use tracing::instrument;

use super::LoginKeyKind;
use crate::Database;
use crate::queries;
use crate::sql::query_count;

/// How many rows of `kind` the login limit holds.
///
/// # Errors
/// - If there's a database error during the query
#[instrument(skip(db), level = "debug")]
pub async fn count_login_limits(db: &Database, kind: LoginKeyKind) -> Result<usize> {
    let conn = db.reader().await?;
    query_count(
        &conn,
        queries::login_limit::COUNT_BY_KIND.bind([kind.as_str().into()]),
    )
    .await
    .context("Failed to count login counts")
}
