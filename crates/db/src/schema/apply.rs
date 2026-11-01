//! Schema versions and their application
//!
//! The database's `user_version` records how many entries of `MIGRATIONS` it has. Each entry
//! is a list of SQL batches; an entry is never changed once released, and a later change to the
//! schema is a new entry. The rule triggers derived from the Rust enums are not versioned: they
//! are dropped and created again on every application.

use anyhow::{Context, Result, bail};
use tracing::instrument;

use super::{bridge, lab, link, node, node_image, user};
use crate::Database;
use crate::sql::query_count;

/// Each entry brings the schema from version `index` to `index + 1`.
const MIGRATIONS: &[&[&str]] = &[&[
    user::TABLES,
    node_image::TABLES,
    lab::TABLES,
    node::TABLES,
    link::TABLES,
    bridge::TABLES,
]];

/// The schema version this build of Sherpa writes.
pub const SCHEMA_VERSION: usize = MIGRATIONS.len();

/// Bring the database to the current schema version and recreate the rule triggers.
///
/// It runs in one transaction, so a failure leaves the database as it was. It is idempotent.
///
/// # Errors
///
/// - If the database has a newer schema version than this build knows, for example after a
///   downgrade of Sherpa
/// - If any statement fails
///
#[instrument(skip(db), level = "debug")]
pub async fn apply_schema(db: &Database) -> Result<()> {
    let mut writer = db.writer().await?;
    let tx = writer
        .conn
        .transaction()
        .await
        .context("Failed to start the schema transaction")?;

    let version = query_count(&tx, "PRAGMA user_version", ())
        .await
        .context("Failed to read the database schema version")?;
    if version > SCHEMA_VERSION {
        bail!(
            "The database has schema version {version}, newer than {SCHEMA_VERSION} that this \
             Sherpa build supports"
        );
    }

    for (index, batches) in MIGRATIONS.iter().enumerate().skip(version) {
        let target = index + 1;
        tracing::info!(from = index, to = target, "Migrating database schema");
        for batch in batches.iter() {
            tx.execute_batch(batch)
                .await
                .context(format!("Failed to migrate the schema to version {target}"))?;
        }
        tx.execute(&format!("PRAGMA user_version = {target}"), ())
            .await
            .context("Failed to record the database schema version")?;
    }

    let rules = [
        user::rules(),
        node_image::rules()?,
        lab::rules()?,
        node::rules()?,
        link::rules()?,
    ];
    for batch in rules.iter() {
        tx.execute_batch(batch)
            .await
            .context("Failed to create the schema rule triggers")?;
    }

    tx.commit()
        .await
        .context("Failed to commit the schema transaction")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::open_in_memory;

    async fn version(db: &Database) -> usize {
        let conn = db.reader().await.unwrap();
        query_count(&conn, "PRAGMA user_version", ()).await.unwrap()
    }

    #[tokio::test]
    async fn apply_schema_sets_the_current_version_and_is_idempotent() {
        let db = open_in_memory().await.unwrap();
        assert_eq!(version(&db).await, 0);

        apply_schema(&db).await.unwrap();
        assert_eq!(version(&db).await, SCHEMA_VERSION);

        apply_schema(&db).await.unwrap();
        assert_eq!(version(&db).await, SCHEMA_VERSION);
    }

    #[tokio::test]
    async fn apply_schema_refuses_a_newer_schema() {
        let db = open_in_memory().await.unwrap();
        apply_schema(&db).await.unwrap();
        let writer = db.writer().await.unwrap();
        writer
            .conn
            .execute(&format!("PRAGMA user_version = {}", SCHEMA_VERSION + 1), ())
            .await
            .unwrap();
        drop(writer);

        let error = apply_schema(&db).await.unwrap_err().to_string();
        assert!(error.contains("newer"), "{error}");
    }

    #[tokio::test]
    async fn apply_schema_creates_the_rule_triggers() {
        let db = open_in_memory().await.unwrap();
        apply_schema(&db).await.unwrap();
        let conn = db.reader().await.unwrap();
        let triggers = query_count(
            &conn,
            "SELECT count(*) FROM sqlite_schema WHERE type = 'trigger'",
            (),
        )
        .await
        .unwrap();
        // Three admin protection triggers, and an insert and an update trigger for each of
        // node_image, lab, node and link.
        assert_eq!(triggers, 3 + 4 * 2);
    }
}
