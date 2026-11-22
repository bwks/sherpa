use std::net::IpAddr;

use anyhow::{Context, Result};
use jiff::Timestamp;
use shared::data::DbUser;
use shared::util::normalize_username;
use tracing::instrument;
use turso::Value;

use crate::Database;
use crate::persistence::{Cells, USER_TABLE, address_bytes, key_of, new_key, user_from_cells};
use crate::queries;
use crate::sql::{execute, query_optional};

/// A login attempt counted against an account that was not locked when it started.
#[derive(Debug, Clone)]
pub struct LoginClaim {
    /// The account, as it was when the attempt was counted.
    pub user: DbUser,
    /// The time this attempt locked the account at, when it was the one that reached the limit.
    lock: Option<String>,
}

impl LoginClaim {
    /// Whether this attempt reached the limit and locked the account.
    #[must_use]
    pub fn locked_account(&self) -> bool {
        self.lock.is_some()
    }
}

/// Count a login attempt against `username` before its password is checked, in one
/// `UPDATE … RETURNING`. The attempt that takes the count to `limit` locks the account.
///
/// Returns `None`, and changes nothing, when the username is unknown or the account is already
/// locked. Because the count is taken before the check, attempts that arrive together can never
/// check more passwords against the account than `limit`.
///
/// # Errors
/// - If there's a database error, or the row cannot be read
#[instrument(skip(db), level = "debug")]
pub async fn claim_login(db: &Database, username: &str, limit: u32) -> Result<Option<LoginClaim>> {
    let writer = db.writer().await?;
    query_optional(
        &writer.conn,
        queries::user::LOGIN_CLAIM.bind([
            normalize_username(username).into(),
            i64::from(limit).into(),
            Timestamp::now().to_string().into(),
        ]),
        |row| {
            let mut cells = Cells::new(row);
            Ok(LoginClaim {
                user: user_from_cells(&mut cells)?,
                lock: cells.optional_text("locked_at")?,
            })
        },
    )
    .await
    .context(format!("Failed to count a login attempt for {username}"))
}

/// Record the result of the attempt `claim` from `address`, and return whether the login
/// stands.
///
/// A login whose password `matched` stands unless another attempt locked the account while its
/// password was being checked; it then sets the count back to 0. Either way the attempt is
/// stored, as a failure unless it stands, in the same transaction.
///
/// # Errors
/// - If there's a database error; nothing is written then
#[instrument(skip(db, claim), fields(username = %claim.user.username), level = "debug")]
pub async fn finish_login(
    db: &Database,
    claim: &LoginClaim,
    matched: bool,
    address: IpAddr,
) -> Result<bool> {
    let id = claim
        .user
        .id
        .as_ref()
        .context("A login claim has a user without an id")?;
    let key = key_of(id, USER_TABLE)?;
    let mut writer = db.writer().await?;
    let tx = writer
        .conn
        .transaction()
        .await
        .context("Failed to start the login result transaction")?;
    let stands = if matched {
        let lock = claim.lock.clone().map_or(Value::Null, Value::from);
        query_optional(
            &tx,
            queries::user::LOGIN_SUCCEEDED.bind([key.as_str().into(), lock]),
            |_| Ok(()),
        )
        .await
        .context(format!("Failed to reset the failed logins of user {id:?}"))?
        .is_some()
    } else {
        false
    };
    execute(
        &tx,
        queries::login_attempt::CREATE.bind([
            new_key().into(),
            key.as_str().into(),
            Timestamp::now().to_string().into(),
            stands.into(),
            Value::Blob(address_bytes(address)),
        ]),
    )
    .await
    .context(format!("Failed to store a login attempt of user {id:?}"))?;
    tx.commit()
        .await
        .context("Failed to commit the login result")?;
    Ok(stands)
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::*;
    use crate::{apply_schema, create_user, open_in_memory};

    const LIMIT: u32 = 3;

    /// The length and hex of each stored address of the only user, oldest first, read as
    /// stored.
    async fn stored_addresses(db: &Database) -> Vec<(i64, String)> {
        let conn = db.reader().await.unwrap();
        let mut rows = conn
            .query(
                "SELECT length(address), hex(address) FROM login_attempt ORDER BY attempted_at",
                (),
            )
            .await
            .unwrap();
        let mut stored = Vec::new();
        while let Some(row) = rows.next().await.unwrap() {
            let length = *row.get_value(0).unwrap().as_integer().unwrap();
            let hex = row.get_value(1).unwrap().as_text().unwrap().clone();
            stored.push((length, hex));
        }
        stored
    }

    #[tokio::test]
    async fn user_login_022_an_ipv6_address_is_stored_as_16_bytes_and_a_mapped_ipv4_one_as_4() {
        let db = open_in_memory().await.unwrap();
        apply_schema(&db).await.unwrap();
        create_user(&db, "alice".into(), "AlicePass123!", false, vec![])
            .await
            .unwrap();
        let addresses = [
            IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x42)),
            IpAddr::V6(Ipv4Addr::new(192, 0, 2, 9).to_ipv6_mapped()),
        ];
        for address in addresses {
            let claim = claim_login(&db, "alice", LIMIT).await.unwrap().unwrap();
            finish_login(&db, &claim, true, address).await.unwrap();
        }

        assert_eq!(
            stored_addresses(&db).await,
            [
                (16, "20010DB8000000000000000000000042".to_owned()),
                (4, "C0000209".to_owned()),
            ]
        );
    }
}
