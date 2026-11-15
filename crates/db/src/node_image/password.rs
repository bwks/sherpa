//! An image's ZTP password, which the `node_image` table stores sealed (see `crate::secret`).
//! Rows are mapped with the sealed text in `ztp.password`; every function that returns an image
//! opens it first, and every write seals it.

use anyhow::{Context, Result};
use shared::data::NodeConfig;
use tracing::instrument;
use turso::Row;

use crate::Database;
use crate::persistence::{Cells, NODE_IMAGE_TABLE, key_of};
use crate::queries;
use crate::secret::{SEALED_TEXT_PREFIX, SealedText, open_text, seal_text};
use crate::sql::{execute, query_all};

/// The place of the image `key`'s password, its associated data.
fn password_context(key: &str) -> String {
    format!("node_image.ztp_password:{key}")
}

/// `image`'s password sealed for the row `key`, or `None` when it has none.
pub(crate) fn seal_password(
    db: &Database,
    image: &NodeConfig,
    key: &str,
) -> Result<Option<SealedText>> {
    match image.ztp.password.as_deref() {
        Some(password) => Ok(Some(seal_text(
            db.secret_key()?,
            password,
            &password_context(key),
        )?)),
        None => Ok(None),
    }
}

/// `image` as stored, with its sealed password opened.
pub(crate) fn open_password(db: &Database, mut image: NodeConfig) -> Result<NodeConfig> {
    if let Some(stored) = image.ztp.password.take() {
        let id = image.id.as_ref().context("A stored node_image has no id")?;
        let key = key_of(id, NODE_IMAGE_TABLE)?;
        image.ztp.password = Some(open_text(
            db.secret_key()?,
            &stored,
            &password_context(&key),
        )?);
    }
    Ok(image)
}

/// `images` as stored, each with its sealed password opened.
pub(crate) fn open_passwords(db: &Database, images: Vec<NodeConfig>) -> Result<Vec<NodeConfig>> {
    images
        .into_iter()
        .map(|image| open_password(db, image))
        .collect()
}

/// A row whose password is plain text: its key and the password.
struct PlainPassword {
    key: String,
    password: String,
}

fn plain_password_row(row: &Row) -> Result<PlainPassword> {
    let mut cells = Cells::new(row);
    Ok(PlainPassword {
        key: cells.text("id")?,
        password: cells.text("ztp_password")?,
    })
}

/// Seal every image password still stored as plain text, as releases before sealing stored
/// them, and return how many. One transaction; a password already sealed is left alone, so a
/// second run changes nothing. sherpad runs it at startup, before anything reads an image.
///
/// # Errors
///
/// Fails when the database has no key, or cannot be read or written.
#[instrument(skip(db))]
pub async fn seal_plain_image_passwords(db: &Database) -> Result<usize> {
    let key = db.secret_key()?;
    let mut writer = db.writer().await?;
    let tx = writer
        .conn
        .transaction()
        .await
        .context("Failed to start the image password transaction")?;
    let plain = query_all(
        &tx,
        queries::node_image::LIST_PLAIN_PASSWORDS.bind([format!("{SEALED_TEXT_PREFIX}*").into()]),
        plain_password_row,
    )
    .await
    .context("Failed to list image passwords stored as plain text")?;
    for row in &plain {
        let sealed = seal_text(key, &row.password, &password_context(&row.key))?;
        execute(
            &tx,
            queries::node_image::UPDATE_ZTP_PASSWORD.bind([
                row.key.as_str().into(),
                sealed.into(),
                row.password.as_str().into(),
            ]),
        )
        .await
        .with_context(|| format!("Failed to seal the password of node_image {}", row.key))?;
    }
    tx.commit()
        .await
        .context("Failed to commit the image password transaction")?;
    Ok(plain.len())
}

#[cfg(test)]
mod tests {
    use shared::data::NodeModel;

    use super::*;
    use crate::{
        DefaultRequest, apply_schema, find_node_image_by_id, list_node_images, open_in_memory,
        update_node_image, upsert_node_image,
    };

    /// The `ztp_password` column of the image `key`, as stored.
    async fn stored_password(db: &Database, key: &str) -> Option<String> {
        let conn = db.reader().await.unwrap();
        let mut rows = conn
            .query("SELECT ztp_password FROM node_image WHERE id = ?1", [key])
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        match row.get_value(0).unwrap() {
            turso::Value::Text(text) => Some(text),
            turso::Value::Null => None,
            other => panic!("ztp_password is {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_image_password_is_sealed_on_write_and_opened_on_read() {
        let db = open_in_memory().await.unwrap();
        apply_schema(&db).await.unwrap();
        let mut image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        image.ztp.password = Some("First-Pass1".to_owned());
        let created = upsert_node_image(&db, image, DefaultRequest::Make)
            .await
            .unwrap();
        assert_eq!(created.ztp.password.as_deref(), Some("First-Pass1"));
        let id = created.id.clone().unwrap();
        let key = key_of(&id, NODE_IMAGE_TABLE).unwrap();
        let first = stored_password(&db, &key).await.unwrap();
        assert!(first.starts_with(SEALED_TEXT_PREFIX), "{first}");
        assert!(!first.contains("First-Pass1"));

        let mut edited = created;
        edited.ztp.password = Some("Second-Pass2".to_owned());
        let updated = update_node_image(&db, edited, DefaultRequest::Keep)
            .await
            .unwrap();
        assert_eq!(updated.ztp.password.as_deref(), Some("Second-Pass2"));
        let second = stored_password(&db, &key).await.unwrap();
        assert!(second.starts_with(SEALED_TEXT_PREFIX) && second != first);
        assert!(!second.contains("Second-Pass2"));

        let read = find_node_image_by_id(&db, id).await.unwrap().unwrap();
        assert_eq!(read.ztp.password.as_deref(), Some("Second-Pass2"));
        let listed = list_node_images(&db).await.unwrap();
        assert_eq!(listed[0].ztp.password.as_deref(), Some("Second-Pass2"));

        // An upsert of the same version again keeps the stored password, sealed once.
        let again = upsert_node_image(
            &db,
            NodeConfig::get_model(NodeModel::UbuntuLinux),
            DefaultRequest::Unset,
        )
        .await;
        assert!(again.is_err(), "the only default cannot be unset");
        let kept = upsert_node_image(
            &db,
            NodeConfig::get_model(NodeModel::UbuntuLinux),
            DefaultRequest::Keep,
        )
        .await
        .unwrap();
        assert_eq!(kept.ztp.password.as_deref(), Some("Second-Pass2"));
    }

    #[tokio::test]
    async fn the_database_refuses_a_plain_image_password() {
        let db = open_in_memory().await.unwrap();
        apply_schema(&db).await.unwrap();
        let created = upsert_node_image(
            &db,
            NodeConfig::get_model(NodeModel::UbuntuLinux),
            DefaultRequest::Make,
        )
        .await
        .unwrap();
        let key = key_of(&created.id.unwrap(), NODE_IMAGE_TABLE).unwrap();
        let writer = db.writer().await.unwrap();
        for plain in [
            "cisco",
            "sealed:short",
            &format!("sealed:{}", "G".repeat(64)),
        ] {
            let error = writer
                .conn
                .execute(
                    "UPDATE node_image SET ztp_password = ?2 WHERE id = ?1",
                    [key.as_str(), plain],
                )
                .await
                .unwrap_err();
            assert!(
                error.to_string().contains("must be sealed"),
                "{plain}: {error}"
            );
        }
        // Other columns of the row can still be changed, and so can the password to NULL.
        writer
            .conn
            .execute(
                "UPDATE node_image SET memory = 4096 WHERE id = ?1",
                [key.as_str()],
            )
            .await
            .unwrap();
        writer
            .conn
            .execute(
                "UPDATE node_image SET ztp_password = NULL WHERE id = ?1",
                [key.as_str()],
            )
            .await
            .unwrap();
    }
}
