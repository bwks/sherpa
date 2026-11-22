use anyhow::{Context, Result};
use tracing::instrument;
use turso::Connection;

use super::{LimitedKey, LoginExpiry, LoginKey};
use crate::Database;
use crate::persistence::login_count_from_row;
use crate::queries;
use crate::sql::{execute, query_count, query_optional};

/// Count an attempt against every key in `keys`, unless one of them is locked or already has
/// as many attempts counted as its limit. Returns, for each key in order, whether it was
/// counted, or `None` when the attempt is refused and nothing was counted.
///
/// Rows that `expiry` says no longer limit anything are deleted first. A key with no row gets
/// one; when its kind already has `max_per_kind` rows, the kind's expired rows are deleted, then
/// its oldest row that is not locked. A lock is never deleted: when every row of the kind is a
/// lock, the key is not counted.
///
/// One transaction under the write lock, so attempts made at the same moment cannot count past
/// a limit.
///
/// # Errors
/// - If there's a database error; nothing is counted then
#[instrument(skip(db, keys), level = "debug")]
pub async fn begin_login_attempt(
    db: &Database,
    keys: &[LimitedKey<'_>],
    expiry: LoginExpiry,
    max_per_kind: usize,
) -> Result<Option<Vec<bool>>> {
    let mut writer = db.writer().await?;
    let tx = writer
        .conn
        .transaction()
        .await
        .context("Failed to start the login attempt transaction")?;

    for limited in keys {
        if !admits(&tx, limited, expiry).await? {
            tx.commit()
                .await
                .context("Failed to commit the login attempt transaction")?;
            return Ok(None);
        }
    }
    let mut counted = Vec::with_capacity(keys.len());
    for limited in keys {
        counted.push(count(&tx, limited.key, expiry, max_per_kind).await?);
    }

    tx.commit()
        .await
        .context("Failed to commit the login attempt transaction")?;
    Ok(Some(counted))
}

/// Whether `limited` may make another attempt, deleting its row first when it has expired.
async fn admits(conn: &Connection, limited: &LimitedKey<'_>, expiry: LoginExpiry) -> Result<bool> {
    let [kind, username, address] = limited.key.values();
    let [now, window_started_by] = expiry.values();
    execute(
        conn,
        queries::login_limit::DELETE_EXPIRED_BY_KEY.bind([
            kind,
            username,
            address,
            now,
            window_started_by,
        ]),
    )
    .await
    .context("Failed to delete an expired login count")?;
    let count = query_optional(
        conn,
        queries::login_limit::FIND.bind(limited.key.values()),
        login_count_from_row,
    )
    .await
    .context("Failed to read a login count")?;
    Ok(count.is_none_or(|count| count.locked_until.is_none() && count.failures < limited.limit))
}

/// Count one attempt for `key`, making room for a new row when its kind is full. `false` when
/// every row of its kind is a lock, so the attempt is not counted for it.
async fn count(
    conn: &Connection,
    key: &LoginKey,
    expiry: LoginExpiry,
    max_per_kind: usize,
) -> Result<bool> {
    let added = execute(conn, queries::login_limit::ADD_FAILURE.bind(key.values()))
        .await
        .context("Failed to count a login attempt")?;
    if added > 0 {
        return Ok(true);
    }
    if !make_room(conn, key, expiry, max_per_kind).await? {
        return Ok(false);
    }
    let [kind, username, address] = key.values();
    execute(
        conn,
        queries::login_limit::CREATE.bind([kind, username, address, expiry.now.as_second().into()]),
    )
    .await
    .context("Failed to count a login attempt")?;
    Ok(true)
}

/// Make room for one more row of `key`'s kind: delete the kind's expired rows, and when that
/// frees nothing, its oldest row that is not locked. `false` when every row is a lock.
async fn make_room(
    conn: &Connection,
    key: &LoginKey,
    expiry: LoginExpiry,
    max_per_kind: usize,
) -> Result<bool> {
    let kind = key.kind().as_str();
    if rows_of_kind(conn, key).await? < max_per_kind {
        return Ok(true);
    }
    let [now, window_started_by] = expiry.values();
    execute(
        conn,
        queries::login_limit::DELETE_EXPIRED_BY_KIND.bind([kind.into(), now, window_started_by]),
    )
    .await
    .context("Failed to delete expired login counts")?;
    if rows_of_kind(conn, key).await? < max_per_kind {
        return Ok(true);
    }
    let dropped = execute(
        conn,
        queries::login_limit::DELETE_OLDEST_UNLOCKED_BY_KIND.bind([kind.into()]),
    )
    .await
    .context("Failed to drop the oldest login count")?;
    Ok(dropped > 0)
}

/// How many rows `key`'s kind has.
async fn rows_of_kind(conn: &Connection, key: &LoginKey) -> Result<usize> {
    query_count(
        conn,
        queries::login_limit::COUNT_BY_KIND.bind([key.kind().as_str().into()]),
    )
    .await
    .context("Failed to count login counts")
}
