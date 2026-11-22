use anyhow::{Context, Result};
use jiff::Timestamp;
use tracing::instrument;
use turso::Connection;

use super::key::Key;
use super::{LoginKeys, LoginRelease, LoginSubject};
use crate::Database;
use crate::sql::{execute, query_optional};

/// Lock until `locked_until` each of `subject`'s pair, address and username keys that is not
/// locked and has as many attempts counted as its limit in `limits`. Returns whether this locked
/// each key.
///
/// One statement per key, each of which reads and changes its row at once.
///
/// # Errors
/// - If there's a database error; keys locked before it stay locked
#[instrument(skip(db, subject), level = "debug")]
pub async fn lock_full_login_keys(
    db: &Database,
    subject: &LoginSubject,
    limits: LoginKeys<u32>,
    locked_until: Timestamp,
) -> Result<LoginKeys<bool>> {
    let writer = db.writer().await?;
    let keys = Key::of(subject);
    Ok(LoginKeys {
        pair: lock_if_full(&writer.conn, keys.pair, limits.pair, locked_until).await?,
        address: lock_if_full(&writer.conn, keys.address, limits.address, locked_until).await?,
        username: lock_if_full(&writer.conn, keys.username, limits.username, locked_until).await?,
    })
}

/// Lock `key` until `locked_until` when it is not locked and has `limit` attempts counted.
/// Whether this locked it.
async fn lock_if_full(
    conn: &Connection,
    key: Key<'_>,
    limit: u32,
    locked_until: Timestamp,
) -> Result<bool> {
    let row = query_optional(conn, key.lock_if_full(limit, locked_until), |_| Ok(()))
        .await
        .context("Failed to lock a login key")?;
    Ok(row.is_some())
}

/// End an attempt of `subject` without it counting: for each of its pair, address and username
/// keys, do what `releases` names. A row given back to no attempts that is not locked is deleted.
///
/// One transaction under the write lock.
///
/// # Errors
/// - If there's a database error; nothing is changed then
#[instrument(skip(db, subject), level = "debug")]
pub async fn give_back_login_attempt(
    db: &Database,
    subject: &LoginSubject,
    releases: LoginKeys<LoginRelease>,
) -> Result<()> {
    let mut writer = db.writer().await?;
    let tx = writer
        .conn
        .transaction()
        .await
        .context("Failed to start the login give-back transaction")?;
    let keys = Key::of(subject);
    for (key, release) in [
        (keys.pair, releases.pair),
        (keys.address, releases.address),
        (keys.username, releases.username),
    ] {
        match release {
            LoginRelease::Keep => {}
            LoginRelease::GiveBack => {
                execute(&tx, key.take_back_failure())
                    .await
                    .context("Failed to give back a login attempt")?;
                execute(&tx, key.delete_if_empty())
                    .await
                    .context("Failed to delete an empty login count")?;
            }
            LoginRelease::Clear => {
                execute(&tx, key.delete_by_key())
                    .await
                    .context("Failed to clear a login count")?;
            }
        }
    }
    tx.commit()
        .await
        .context("Failed to commit the login give-back transaction")
}
