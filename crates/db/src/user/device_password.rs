//! A user's device password, their login password on lab nodes, which the `user` table stores
//! sealed (see `crate::secret`). The password is not part of `DbUser`: it is written with
//! [`set_device_password`], tested with [`device_password_set`] and read only with
//! [`open_device_password`], which returns it in a [`DevicePassword`].

use std::fmt;

use anyhow::{Context, Result, anyhow};
use jiff::Timestamp;
use shared::data::RecordId;
use tracing::instrument;
use turso::Row;

use crate::Database;
use crate::persistence::{Cells, USER_TABLE, key_of};
use crate::queries;
use crate::secret::{open_text, seal_text};
use crate::sql::query_optional;

/// A user's device password, opened. It has no `Serialize`, so no response can carry it, and
/// its `Debug` shows nothing of it.
#[derive(Clone, PartialEq, Eq)]
pub struct DevicePassword(String);

impl DevicePassword {
    /// The password, for the node configuration that sets it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for DevicePassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DevicePassword([REDACTED])")
    }
}

/// The place of the user `key`'s device password, its associated data.
fn device_password_context(key: &str) -> String {
    format!("user.device_password:{key}")
}

fn key_from_row(row: &Row) -> Result<String> {
    Cells::new(row).text("id")
}

fn is_set_from_row(row: &Row) -> Result<bool> {
    Cells::new(row).boolean("device_password_set")
}

fn stored_from_row(row: &Row) -> Result<Option<String>> {
    Cells::new(row).optional_text("device_password")
}

/// Set the device password of user `id` to `password`, sealed for the user's row, replacing any
/// it had. It also sets the user's `updated_at`. The caller checks who may set it and that the
/// password meets the rule; this only stores it.
///
/// # Errors
/// - If `id` is not a user id, or the user does not exist
/// - If the database has no key, or cannot be written
#[instrument(skip(db, password), fields(user = %id), level = "debug")]
pub async fn set_device_password(db: &Database, id: &RecordId, password: &str) -> Result<()> {
    let key = key_of(id, USER_TABLE)?;
    let sealed = seal_text(db.secret_key()?, password, &device_password_context(&key))?;
    let writer = db.writer().await?;
    query_optional(
        &writer.conn,
        queries::user::UPDATE_DEVICE_PASSWORD.bind([
            key.as_str().into(),
            sealed.into(),
            Timestamp::now().to_string().into(),
        ]),
        key_from_row,
    )
    .await
    .with_context(|| format!("Failed to set the device password of user {id}"))?
    .ok_or_else(|| anyhow!("User not found for device password change: {id}"))?;
    Ok(())
}

/// Whether user `id` has a device password, without opening it.
///
/// # Errors
/// - If `id` is not a user id, or the user does not exist
/// - If the database cannot be read
#[instrument(skip(db), fields(user = %id), level = "debug")]
pub async fn device_password_set(db: &Database, id: &RecordId) -> Result<bool> {
    let key = key_of(id, USER_TABLE)?;
    let conn = db.reader().await?;
    query_optional(
        &conn,
        queries::user::DEVICE_PASSWORD_SET.bind([key.into()]),
        is_set_from_row,
    )
    .await
    .with_context(|| format!("Failed to read whether user {id} has a device password"))?
    .ok_or_else(|| anyhow!("User not found: {id}"))
}

/// The device password of user `id`, opened, or `None` when they have not set one. Only the
/// places that build a node's configuration call it.
///
/// # Errors
/// - If `id` is not a user id, or the user does not exist
/// - If the database has no key, or cannot be read
/// - If the stored value does not open: it is not sealed, was sealed under another key or for
///   another row, or has been changed
#[instrument(skip(db), fields(user = %id), level = "debug")]
pub async fn open_device_password(db: &Database, id: &RecordId) -> Result<Option<DevicePassword>> {
    let key = key_of(id, USER_TABLE)?;
    let conn = db.reader().await?;
    let stored = query_optional(
        &conn,
        queries::user::FIND_DEVICE_PASSWORD.bind([key.as_str().into()]),
        stored_from_row,
    )
    .await
    .with_context(|| format!("Failed to read the device password of user {id}"))?
    .ok_or_else(|| anyhow!("User not found: {id}"))?;
    stored
        .map(|text| {
            open_text(db.secret_key()?, &text, &device_password_context(&key)).map(DevicePassword)
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::{SEALED_TEXT_PREFIX, SecretKey};
    use crate::{apply_schema, create_user, get_user, open_in_memory};

    const DEVICE_PASSWORD: &str = "Granite-Owl42";

    async fn database_with(usernames: &[&str]) -> (Database, Vec<RecordId>) {
        let db = open_in_memory().await.unwrap();
        apply_schema(&db).await.unwrap();
        let mut ids = Vec::new();
        for username in usernames {
            let user = create_user(&db, (*username).to_owned(), "TestPass123!", false, vec![])
                .await
                .unwrap();
            ids.push(user.id.unwrap());
        }
        (db, ids)
    }

    /// The `device_password` column of user `id`, as stored.
    async fn stored(db: &Database, id: &RecordId) -> Option<String> {
        let key = key_of(id, USER_TABLE).unwrap();
        let conn = db.reader().await.unwrap();
        let mut rows = conn
            .query("SELECT device_password FROM user WHERE id = ?1", [key])
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        match row.get_value(0).unwrap() {
            turso::Value::Text(text) => Some(text),
            turso::Value::Null => None,
            other => panic!("device_password is {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_device_password_is_sealed_on_write_and_opened_on_read() {
        let (db, ids) = database_with(&["alice"]).await;
        let alice = &ids[0];
        assert!(!device_password_set(&db, alice).await.unwrap());
        assert_eq!(open_device_password(&db, alice).await.unwrap(), None);
        assert_eq!(stored(&db, alice).await, None);

        set_device_password(&db, alice, DEVICE_PASSWORD)
            .await
            .unwrap();
        assert!(device_password_set(&db, alice).await.unwrap());
        let first = stored(&db, alice).await.unwrap();
        assert!(first.starts_with(SEALED_TEXT_PREFIX), "{first}");
        assert!(!first.contains(DEVICE_PASSWORD));
        let opened = open_device_password(&db, alice).await.unwrap().unwrap();
        assert_eq!(opened.expose(), DEVICE_PASSWORD);

        // A new password replaces it, sealed afresh.
        set_device_password(&db, alice, "Second-Kite77")
            .await
            .unwrap();
        let second = stored(&db, alice).await.unwrap();
        assert_ne!(first, second);
        let opened = open_device_password(&db, alice).await.unwrap().unwrap();
        assert_eq!(opened.expose(), "Second-Kite77");
    }

    #[tokio::test]
    async fn the_user_record_does_not_carry_the_device_password() {
        let (db, ids) = database_with(&["alice"]).await;
        set_device_password(&db, &ids[0], DEVICE_PASSWORD)
            .await
            .unwrap();
        let user = get_user(&db, "alice").await.unwrap();
        let printed = format!("{user:?}");
        assert!(!printed.contains(DEVICE_PASSWORD), "{printed}");
        assert!(!printed.contains(SEALED_TEXT_PREFIX), "{printed}");
    }

    #[tokio::test]
    async fn an_opened_device_password_prints_redacted() {
        let (db, ids) = database_with(&["alice"]).await;
        set_device_password(&db, &ids[0], DEVICE_PASSWORD)
            .await
            .unwrap();
        let opened = open_device_password(&db, &ids[0]).await.unwrap().unwrap();
        let printed = format!("{opened:?} {:?}", Some(&opened));
        assert!(!printed.contains(DEVICE_PASSWORD), "{printed}");
        assert_eq!(format!("{opened:?}"), "DevicePassword([REDACTED])");
    }

    #[tokio::test]
    async fn a_sealed_device_password_opens_only_for_its_own_row_and_key() {
        let (db, ids) = database_with(&["alice", "bob"]).await;
        let (alice, bob) = (&ids[0], &ids[1]);
        set_device_password(&db, alice, DEVICE_PASSWORD)
            .await
            .unwrap();
        // Alice's sealed value copied to Bob's row passes the triggers but does not open there.
        let sealed = stored(&db, alice).await.unwrap();
        {
            let writer = db.writer().await.unwrap();
            writer
                .conn
                .execute(
                    "UPDATE user SET device_password = ?2 WHERE id = ?1",
                    [key_of(bob, USER_TABLE).unwrap(), sealed],
                )
                .await
                .unwrap();
        }
        assert!(device_password_set(&db, bob).await.unwrap());
        assert!(open_device_password(&db, bob).await.is_err());
        // Under another database key, Alice's does not open either.
        let other = db.clone().with_secret_key(SecretKey::generate());
        assert!(open_device_password(&other, alice).await.is_err());
    }

    #[tokio::test]
    async fn the_database_refuses_a_plain_device_password() {
        let (db, ids) = database_with(&["alice"]).await;
        let key = key_of(&ids[0], USER_TABLE).unwrap();
        let writer = db.writer().await.unwrap();
        for plain in [
            DEVICE_PASSWORD,
            "sealed:short",
            &format!("sealed:{}", "G".repeat(64)),
        ] {
            let error = writer
                .conn
                .execute(
                    "UPDATE user SET device_password = ?2 WHERE id = ?1",
                    [key.as_str(), plain],
                )
                .await
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("user.device_password must be sealed"),
                "{plain}: {error}"
            );
            let error = writer
                .conn
                .execute(
                    "INSERT INTO user (id, username, password_hash, is_admin, ssh_keys, \
                     created_at, updated_at, device_password) VALUES ('raw', 'raw', 'x', 0, \
                     '[]', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', ?1)",
                    [plain],
                )
                .await
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("user.device_password must be sealed"),
                "{plain}: {error}"
            );
        }
    }
}
