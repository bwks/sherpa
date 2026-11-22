use anyhow::{Context, Result};
use jiff::Timestamp;
use tracing::instrument;

use super::{LimitedKey, LoginKey};
use crate::Database;
use crate::queries;
use crate::sql::{execute, query_optional};

/// Lock until `locked_until` each key in `keys` that is not locked and has as many attempts
/// counted as its limit. Returns, for each key in order, whether this locked it.
///
/// One statement per key, each of which reads and changes its row at once.
///
/// # Errors
/// - If there's a database error; keys locked before it stay locked
#[instrument(skip(db, keys), level = "debug")]
pub async fn lock_full_login_keys(
    db: &Database,
    keys: &[LimitedKey<'_>],
    locked_until: Timestamp,
) -> Result<Vec<bool>> {
    let writer = db.writer().await?;
    let mut locked = Vec::with_capacity(keys.len());
    for limited in keys {
        let [kind, username, address] = limited.key.values();
        let row = query_optional(
            &writer.conn,
            queries::login_limit::LOCK_IF_FULL.bind([
                kind,
                username,
                address,
                i64::from(limited.limit).into(),
                locked_until.as_second().into(),
            ]),
            |_| Ok(()),
        )
        .await
        .context("Failed to lock a login key")?;
        locked.push(row.is_some());
    }
    Ok(locked)
}

/// End an attempt without it counting: delete the rows of `cleared`, and take one attempt back
/// from each of `given_back`, deleting a row left with none that is not locked.
///
/// One transaction under the write lock.
///
/// # Errors
/// - If there's a database error; nothing is changed then
#[instrument(skip(db, cleared, given_back), level = "debug")]
pub async fn give_back_login_attempt(
    db: &Database,
    cleared: &[&LoginKey],
    given_back: &[&LoginKey],
) -> Result<()> {
    let mut writer = db.writer().await?;
    let tx = writer
        .conn
        .transaction()
        .await
        .context("Failed to start the login give-back transaction")?;
    for key in cleared {
        execute(&tx, queries::login_limit::DELETE_BY_KEY.bind(key.values()))
            .await
            .context("Failed to clear a login count")?;
    }
    for key in given_back {
        execute(
            &tx,
            queries::login_limit::TAKE_BACK_FAILURE.bind(key.values()),
        )
        .await
        .context("Failed to give back a login attempt")?;
        execute(
            &tx,
            queries::login_limit::DELETE_IF_EMPTY.bind(key.values()),
        )
        .await
        .context("Failed to delete an empty login count")?;
    }
    tx.commit()
        .await
        .context("Failed to commit the login give-back transaction")
}
