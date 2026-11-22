use anyhow::{Context, Result};
use tracing::instrument;
use turso::Connection;

use super::key::Key;
use super::{LoginExpiry, LoginKeys, LoginSubject};
use crate::Database;
use crate::persistence::{login_count_from_row, login_limit_sizes_from_row};
use crate::queries;
use crate::sql::{execute, query_optional};

/// Count an attempt against each of `subject`'s pair, address and username keys, unless one of
/// them is locked or already has as many attempts counted as its limit in `limits`. Returns
/// whether each key was counted, or `None` when the attempt is refused and nothing was counted.
///
/// Rows that `expiry` says no longer limit anything are deleted first. A key with no row gets
/// one; when its table already has `max_per_table` rows, the table's expired rows are deleted,
/// then its oldest row that is not locked. A lock is never deleted: when every row of the table
/// is a lock, the key is not counted.
///
/// One transaction under the write lock, so attempts made at the same moment cannot count past
/// a limit.
///
/// # Errors
/// - If there's a database error; nothing is counted then
#[instrument(skip(db, subject), level = "debug")]
pub async fn begin_login_attempt(
    db: &Database,
    subject: &LoginSubject,
    limits: LoginKeys<u32>,
    expiry: LoginExpiry,
    max_per_table: usize,
) -> Result<Option<LoginKeys<bool>>> {
    let mut writer = db.writer().await?;
    let tx = writer
        .conn
        .transaction()
        .await
        .context("Failed to start the login attempt transaction")?;

    let keys = Key::of(subject);
    for (key, limit) in [
        (keys.pair, limits.pair),
        (keys.address, limits.address),
        (keys.username, limits.username),
    ] {
        if !admits(&tx, key, limit, expiry).await? {
            tx.commit()
                .await
                .context("Failed to commit the login attempt transaction")?;
            return Ok(None);
        }
    }
    let counted = LoginKeys {
        pair: count(&tx, keys.pair, expiry, max_per_table).await?,
        address: count(&tx, keys.address, expiry, max_per_table).await?,
        username: count(&tx, keys.username, expiry, max_per_table).await?,
    };

    tx.commit()
        .await
        .context("Failed to commit the login attempt transaction")?;
    Ok(Some(counted))
}

/// Whether `key` may make another attempt under `limit`, deleting its row first when it has
/// expired.
async fn admits(conn: &Connection, key: Key<'_>, limit: u32, expiry: LoginExpiry) -> Result<bool> {
    execute(conn, key.delete_expired_by_key(expiry))
        .await
        .context("Failed to delete an expired login count")?;
    let count = query_optional(conn, key.find(), login_count_from_row)
        .await
        .context("Failed to read a login count")?;
    Ok(count.is_none_or(|count| count.locked_until.is_none() && count.failures < limit))
}

/// Count one attempt for `key`, making room for a new row when its table is full. `false` when
/// every row of its table is a lock, so the attempt is not counted for it.
async fn count(
    conn: &Connection,
    key: Key<'_>,
    expiry: LoginExpiry,
    max_per_table: usize,
) -> Result<bool> {
    let added = execute(conn, key.add_failure())
        .await
        .context("Failed to count a login attempt")?;
    if added > 0 {
        return Ok(true);
    }
    if !make_room(conn, key, expiry, max_per_table).await? {
        return Ok(false);
    }
    execute(conn, key.create(expiry.now))
        .await
        .context("Failed to count a login attempt")?;
    Ok(true)
}

/// Make room for one more row in `key`'s table: delete the table's expired rows, and when that
/// frees nothing, its oldest row that is not locked. `false` when every row is a lock.
async fn make_room(
    conn: &Connection,
    key: Key<'_>,
    expiry: LoginExpiry,
    max_per_table: usize,
) -> Result<bool> {
    if key.rows(sizes(conn).await?) < max_per_table {
        return Ok(true);
    }
    execute(conn, key.delete_expired(expiry))
        .await
        .context("Failed to delete expired login counts")?;
    if key.rows(sizes(conn).await?) < max_per_table {
        return Ok(true);
    }
    let dropped = execute(conn, key.delete_oldest_unlocked())
        .await
        .context("Failed to drop the oldest login count")?;
    Ok(dropped > 0)
}

/// How many rows each login limit table has.
pub(super) async fn sizes(conn: &Connection) -> Result<LoginKeys<usize>> {
    query_optional(
        conn,
        queries::login_limit_size::FIND.bind([]),
        login_limit_sizes_from_row,
    )
    .await
    .context("Failed to count login counts")?
    .context("The login limit sizes row is missing")
}
