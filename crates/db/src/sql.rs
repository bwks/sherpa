//! Query helpers shared by the table modules.
use anyhow::{Context, Result};
use tracing::instrument;
use turso::{Connection, Row, Value};

/// A statement that binds `N` parameters, `?1` to `?N`.
///
/// `build.rs` generates one per query file, with the count it read from the file, as
/// `queries::<table>::<NAME>`. The helpers below run only a [`Bound`] query, and
/// [`Query::bind`] takes exactly `N` values, so binding a different number does not compile:
///
/// ```compile_fail,E0308
/// use db::Query;
/// use turso::Value;
///
/// const FIND: Query<2> = Query::new("SELECT id FROM node WHERE name = ?1 AND lab = ?2");
/// let _ = FIND.bind([Value::from("router1")]);
/// ```
///
/// The right number does:
///
/// ```
/// use db::Query;
/// use turso::Value;
///
/// const FIND: Query<2> = Query::new("SELECT id FROM node WHERE name = ?1 AND lab = ?2");
/// let _ = FIND.bind([Value::from("router1"), Value::from("lab1")]);
/// ```
#[derive(Clone, Copy, Debug)]
pub struct Query<const N: usize> {
    sql: &'static str,
}

/// A query with its parameters, ready to run.
#[derive(Debug)]
pub struct Bound {
    sql: &'static str,
    params: Vec<Value>,
}

impl<const N: usize> Query<N> {
    /// A statement binding `N` parameters. The count is taken as given: the generated queries
    /// get theirs from `build.rs`, which counts the placeholders.
    #[must_use]
    pub const fn new(sql: &'static str) -> Self {
        Self { sql }
    }

    /// Bind the statement's `N` parameters, `params[0]` to `?1` and so on.
    #[must_use]
    pub fn bind(self, params: [Value; N]) -> Bound {
        Bound {
            sql: self.sql,
            params: params.into(),
        }
    }
}

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
#[instrument(skip(conn, query, map), level = "debug")]
pub(crate) async fn query_optional<T>(
    conn: &Connection,
    query: Bound,
    map: fn(&Row) -> Result<T>,
) -> Result<Option<T>> {
    let Bound { sql, params } = query;
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
#[instrument(skip(conn, query, map), level = "debug")]
pub(crate) async fn query_all<T>(
    conn: &Connection,
    query: Bound,
    map: fn(&Row) -> Result<T>,
) -> Result<Vec<T>> {
    let Bound { sql, params } = query;
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
#[instrument(skip(conn, query), level = "debug")]
pub(crate) async fn query_count(conn: &Connection, query: Bound) -> Result<usize> {
    let Bound { sql, params } = query;
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

/// Run a statement that returns no rows, and return how many rows it changed.
#[instrument(skip(conn, query), level = "debug")]
pub(crate) async fn execute(conn: &Connection, query: Bound) -> Result<u64> {
    let Bound { sql, params } = query;
    conn.execute(sql, params)
        .await
        .with_context(|| format!("Statement failed: {}", statement(sql)))
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
        let error = query_all(
            &conn,
            Query::new("SELECT id FROM t\nORDER BY id").bind([]),
            fail_on_b,
        )
        .await
        .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("row b"), "{message}");
        assert!(message.contains("SELECT id FROM t"), "{message}");

        let error = query_count(&conn, Query::new("SELECT count(*) FROM missing").bind([]))
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
