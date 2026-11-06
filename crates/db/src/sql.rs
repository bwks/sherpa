//! Query helpers shared by the table modules.
use anyhow::{Context, Result};
use tracing::instrument;
use turso::{Connection, IntoParams, Row, Value};

/// Quote `text` as an SQL string literal, for the few places SQL takes no bound parameter:
/// trigger bodies and `VACUUM INTO`.
pub(crate) fn sql_text(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// The first line of `sql`, which names the statement in errors.
fn statement(sql: &str) -> &str {
    sql.lines().next().unwrap_or_default().trim()
}

/// The first column of `row`, which is its id in every table, to name it in errors.
fn row_name(row: &Row) -> String {
    match row.get_value(0) {
        Ok(Value::Text(text)) => text,
        Ok(other) => format!("{other:?}"),
        Err(_) => "?".to_owned(),
    }
}

/// Run a query and map its first row, if there is one.
#[instrument(skip(conn, params, map), level = "debug")]
pub(crate) async fn query_optional<T>(
    conn: &Connection,
    sql: &str,
    params: impl IntoParams,
    map: fn(&Row) -> Result<T>,
) -> Result<Option<T>> {
    let mut rows = conn
        .query(sql, params)
        .await
        .with_context(|| format!("Query failed: {}", statement(sql)))?;
    match rows.next().await.context("Failed to read query result")? {
        Some(row) => Ok(Some(map(&row).with_context(|| {
            format!(
                "Failed to read row {} of: {}",
                row_name(&row),
                statement(sql)
            )
        })?)),
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
    let mut rows = conn
        .query(sql, params)
        .await
        .with_context(|| format!("Query failed: {}", statement(sql)))?;
    let mut mapped = Vec::new();
    while let Some(row) = rows.next().await.context("Failed to read query result")? {
        mapped.push(map(&row).with_context(|| {
            format!(
                "Failed to read row {} of: {}",
                row_name(&row),
                statement(sql)
            )
        })?);
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
    let mut rows = conn
        .query(sql, params)
        .await
        .with_context(|| format!("Query failed: {}", statement(sql)))?;
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

    fn fail_on_b(row: &Row) -> Result<String> {
        match row.get_value(0)? {
            turso::Value::Text(id) if id == "b" => anyhow::bail!("cannot decode"),
            turso::Value::Text(id) => Ok(id),
            other => anyhow::bail!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_row_that_does_not_decode_is_named_with_its_statement() {
        let db = crate::open_in_memory().await.unwrap();
        let conn = db.reader().await.unwrap();
        conn.execute_batch("CREATE TABLE t (id TEXT); INSERT INTO t VALUES ('a'), ('b');")
            .await
            .unwrap();
        let error = query_all(&conn, "SELECT id FROM t\nORDER BY id", (), fail_on_b)
            .await
            .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("row b"), "{message}");
        assert!(message.contains("SELECT id FROM t"), "{message}");

        let error = query_count(&conn, "SELECT count(*) FROM missing", ())
            .await
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("SELECT count(*) FROM missing"),
            "{error:#}"
        );
    }

    #[test]
    fn sql_text_escapes_quotes() {
        assert_eq!(sql_text("it's"), "'it''s'");
    }
}
