//! Query helpers shared by the table modules.
use anyhow::{Context, Result};
use tracing::instrument;
use turso::{Connection, IntoParams, Row};

/// Quote `text` as an SQL string literal, for the few places SQL takes no bound parameter:
/// trigger bodies and `VACUUM INTO`.
pub(crate) fn sql_text(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Run a query and map its first row, if there is one.
#[instrument(skip(conn, params, map), level = "debug")]
pub(crate) async fn query_optional<T>(
    conn: &Connection,
    sql: &str,
    params: impl IntoParams,
    map: fn(&Row) -> Result<T>,
) -> Result<Option<T>> {
    let mut rows = conn.query(sql, params).await.context("Query failed")?;
    match rows.next().await.context("Failed to read query result")? {
        Some(row) => Ok(Some(map(&row)?)),
        None => Ok(None),
    }
}

/// Run a query and map every row.
#[instrument(skip(conn, params, map), level = "debug")]
pub(crate) async fn query_all<T>(
    conn: &Connection,
    sql: &str,
    params: impl IntoParams,
    map: fn(&Row) -> Result<T>,
) -> Result<Vec<T>> {
    let mut rows = conn.query(sql, params).await.context("Query failed")?;
    let mut mapped = Vec::new();
    while let Some(row) = rows.next().await.context("Failed to read query result")? {
        mapped.push(map(&row)?);
    }
    Ok(mapped)
}

/// Run a query whose first column of its first row is a count.
#[instrument(skip(conn, params), level = "debug")]
pub(crate) async fn query_count(
    conn: &Connection,
    sql: &str,
    params: impl IntoParams,
) -> Result<usize> {
    let mut rows = conn.query(sql, params).await.context("Query failed")?;
    let row = rows
        .next()
        .await
        .context("Failed to read query result")?
        .context("Count query returned no row")?;
    let count = row
        .get_value(0)
        .context("Count query returned no column")?
        .as_integer()
        .copied()
        .context("Count query returned a non-integer")?;
    usize::try_from(count).context("Count query returned a negative count")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_text_escapes_quotes() {
        assert_eq!(sql_text("it's"), "'it''s'");
    }
}
