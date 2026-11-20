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
    use crate::secret::{SEALED_TEXT_PREFIX, SecretKey, open, seal, seal_text};
    use crate::{apply_schema, create_user, open_in_memory};
    use shared::data::SetDevicePasswordRequest;
    use turso::Value;

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
    async fn device_password_009_setting_it_again_replaces_it() {
        let (db, ids) = database_with(&["alice"]).await;
        let alice = &ids[0];
        set_device_password(&db, alice, DEVICE_PASSWORD)
            .await
            .unwrap();
        let first = stored(&db, alice).await.unwrap();

        set_device_password(&db, alice, "Second-Kite77")
            .await
            .unwrap();
        let second = stored(&db, alice).await.unwrap();
        assert_ne!(first, second);
        let opened = open_device_password(&db, alice).await.unwrap().unwrap();
        assert_eq!(opened.expose(), "Second-Kite77");
    }

    #[tokio::test]
    async fn device_password_011_it_is_sealed_for_its_row_and_key_and_a_plain_value_is_refused() {
        let (db, ids) = database_with(&["alice", "bob"]).await;
        let (alice, bob) = (&ids[0], &ids[1]);
        assert!(!device_password_set(&db, alice).await.unwrap());
        assert_eq!(open_device_password(&db, alice).await.unwrap(), None);
        assert_eq!(stored(&db, alice).await, None);

        set_device_password(&db, alice, DEVICE_PASSWORD)
            .await
            .unwrap();
        assert!(device_password_set(&db, alice).await.unwrap());
        let sealed = stored(&db, alice).await.unwrap();
        assert!(sealed.starts_with(SEALED_TEXT_PREFIX), "{sealed}");
        assert!(!sealed.contains(DEVICE_PASSWORD));
        let opened = open_device_password(&db, alice).await.unwrap().unwrap();
        assert_eq!(opened.expose(), DEVICE_PASSWORD);

        // Alice's sealed value copied to Bob's row passes the triggers but does not open there.
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

        // The sealing itself: a value opens only with its key and place, and a text is sealed
        // as prefixed lower-case hex.
        {
            let key = SecretKey::generate();
            let place = "user.device_password:abc";
            let sealed = seal(&key, b"hunter2", place).unwrap();
            assert!(!sealed.windows(7).any(|window| window == b"hunter2"));
            assert_eq!(open(&key, &sealed, place).unwrap(), b"hunter2");
            assert!(open(&key, &sealed, "user.device_password:xyz").is_err());
            let error = open(&SecretKey::generate(), &sealed, place).unwrap_err();
            assert!(
                error.to_string().contains("different database key"),
                "{error}"
            );

            let sealed = seal_text(&key, "cisco", place).unwrap();
            let Value::Text(text) = Value::from(sealed) else {
                panic!("sealed text is not text");
            };
            let hex = text.strip_prefix(SEALED_TEXT_PREFIX).unwrap();
            assert!(hex::decode(hex).is_ok(), "{text}");
            assert_eq!(hex, hex.to_lowercase(), "{text}");
            assert!(!text.contains("cisco"));
            assert_eq!(open_text(&key, &text, place).unwrap(), "cisco");
            for stored in ["cisco", "sealed:zz", "sealed:abc", "sealed:00"] {
                assert!(open_text(&key, stored, "place").is_err(), "{stored}");
            }
            let error = open_text(&key, "cisco", "place").unwrap_err();
            assert!(error.to_string().contains("not sealed"), "{error}");
            assert!(open_text(&key, &text, "user.device_password:other").is_err());
        }

        let key = key_of(alice, USER_TABLE).unwrap();
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

    #[tokio::test]
    async fn device_password_014_a_device_password_shows_nothing_of_itself_when_printed() {
        let request = SetDevicePasswordRequest {
            username: "alice".to_owned(),
            login_password: "Login-Pass-123!".to_owned(),
            device_password: DEVICE_PASSWORD.to_owned(),
            token: "token-value".to_owned(),
        };
        let printed = format!("{request:?}");
        assert!(printed.contains("alice"), "{printed}");
        for secret in ["Login-Pass-123!", DEVICE_PASSWORD, "token-value"] {
            assert!(!printed.contains(secret), "{printed}");
        }

        let (db, ids) = database_with(&["alice"]).await;
        set_device_password(&db, &ids[0], DEVICE_PASSWORD)
            .await
            .unwrap();
        let opened = open_device_password(&db, &ids[0]).await.unwrap().unwrap();
        let printed = format!("{opened:?} {:?}", Some(&opened));
        assert!(!printed.contains(DEVICE_PASSWORD), "{printed}");
        assert_eq!(format!("{opened:?}"), "DevicePassword([REDACTED])");
    }
}
