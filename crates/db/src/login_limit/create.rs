use anyhow::{Context, Result};
use tracing::instrument;
use turso::Connection;

use super::read::rows;
use super::{LoginAdmission, LoginFailureId, LoginKeys, LoginSubject, LoginWindow};
use crate::Database;
use crate::persistence::{login_counts_from_row, rowid_from_row};
use crate::queries;
use crate::sql::{execute, query_optional};

/// Start an attempt of `subject`: count the failures in `window` for its pair, its address and
/// its username, and refuse it when one of them has as many as its limit in `limits`. An admitted
/// attempt writes its own failure row at `window.now`, which stays if it fails.
///
/// The table holds at most `max_rows` rows. At the cap the expired rows are deleted first, and
/// when that frees nothing the attempt writes no row; a live row is never deleted. A refused
/// attempt writes nothing.
///
/// One transaction under the write lock, so attempts made at the same moment cannot get past a
/// limit.
///
/// # Errors
/// - If there's a database error; nothing is written then
#[instrument(skip(db, subject), level = "debug")]
pub async fn begin_login_attempt(
    db: &Database,
    subject: &LoginSubject,
    limits: LoginKeys<u32>,
    window: LoginWindow,
    max_rows: usize,
) -> Result<LoginAdmission> {
    let mut writer = db.writer().await?;
    let tx = writer
        .conn
        .transaction()
        .await
        .context("Failed to start the login attempt transaction")?;

    let counts = query_optional(
        &tx,
        queries::login_failure::COUNT_BY_SUBJECT.bind([
            subject.username.as_str().into(),
            subject.address.value(),
            window.expired_by.as_second().into(),
        ]),
        login_counts_from_row,
    )
    .await
    .context("Failed to count failed logins")?
    .context("The failed login count returned no row")?;
    let admission = if counts.pair >= limits.pair
        || counts.address >= limits.address
        || counts.username >= limits.username
    {
        LoginAdmission::Refused
    } else if has_room(&tx, window, max_rows).await? {
        let rowid = query_optional(
            &tx,
            queries::login_failure::CREATE.bind([
                subject.username.as_str().into(),
                subject.address.value(),
                window.now.as_second().into(),
            ]),
            rowid_from_row,
        )
        .await
        .context("Failed to write the failed login")?
        .context("Writing the failed login returned no row")?;
        LoginAdmission::Admitted {
            counts: LoginKeys {
                pair: counts.pair + 1,
                address: counts.address + 1,
                username: counts.username + 1,
            },
            failure: Some(LoginFailureId::new(rowid, window.now)),
        }
    } else {
        LoginAdmission::Admitted {
            counts,
            failure: None,
        }
    };

    tx.commit()
        .await
        .context("Failed to commit the login attempt transaction")?;
    Ok(admission)
}

/// Whether the table has room for one more row under `max_rows`, deleting the rows `window` says
/// have expired when it is at the cap.
async fn has_room(conn: &Connection, window: LoginWindow, max_rows: usize) -> Result<bool> {
    if rows(conn).await? < max_rows {
        return Ok(true);
    }
    execute(
        conn,
        queries::login_failure::DELETE_EXPIRED.bind([window.expired_by.as_second().into()]),
    )
    .await
    .context("Failed to delete expired failed logins")?;
    Ok(rows(conn).await? < max_rows)
}
