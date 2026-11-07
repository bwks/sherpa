//! Schema versions and their application
//!
//! The database's `user_version` is its schema version. Version 1 is the tables in `sql/schema/`;
//! each file in `sql/migrations/` takes the schema up one version, in order. A released version
//! is never changed, and a later change to the schema is a new migration file.

use anyhow::{Context, Result, bail};
use tracing::instrument;

use super::{bridge, lab, link, migrations, node, node_image, user};
use crate::Database;
use crate::sql::query_count;

/// Schema version 1: every table, parents before children.
const TABLES: &[&str] = &[
    user::TABLES,
    node_image::TABLES,
    lab::TABLES,
    node::TABLES,
    link::TABLES,
    bridge::TABLES,
];

/// The schema version this build of Sherpa writes.
pub const SCHEMA_VERSION: usize = 1 + migrations::ALL.len();

/// Bring the database to the current schema version.
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

    for target in (version + 1)..=SCHEMA_VERSION {
        tracing::info!(from = target - 1, to = target, "Migrating database schema");
        let batches = match target {
            1 => TABLES,
            _ => migrations::ALL
                .get(target - 2..target - 1)
                .context(format!("Schema version {target} has no migration"))?,
        };
        for batch in batches {
            tx.execute_batch(batch)
                .await
                .context(format!("Failed to migrate the schema to version {target}"))?;
        }
        tx.execute(&format!("PRAGMA user_version = {target}"), ())
            .await
            .context("Failed to record the database schema version")?;
    }

    tx.commit()
        .await
        .context("Failed to commit the schema transaction")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use shared::data::{DiskBuses, NodeConfig, NodeModel, ZtpMethod};

    use super::*;
    use crate::{list_node_images, open_in_memory, upsert_node_image};

    async fn version(db: &Database) -> usize {
        let conn = db.reader().await.unwrap();
        query_count(&conn, "PRAGMA user_version", ()).await.unwrap()
    }

    async fn trigger_count(db: &Database, pattern: &str) -> usize {
        let conn = db.reader().await.unwrap();
        query_count(
            &conn,
            "SELECT count(*) FROM sqlite_schema WHERE type = 'trigger' AND name LIKE ?1",
            [pattern],
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_value_a_newer_release_stored_is_kept_and_named_when_read() {
        let db = open_in_memory().await.unwrap();
        apply_schema(&db).await.unwrap();
        upsert_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux))
            .await
            .unwrap();
        // As a newer release with a model this build does not know would leave the row.
        {
            let writer = db.writer().await.unwrap();
            writer
                .conn
                .execute("UPDATE node_image SET model = 'future_model'", ())
                .await
                .unwrap();
        }

        apply_schema(&db).await.unwrap();
        let error = list_node_images(&db).await.unwrap_err();
        assert!(
            format!("{error:#}").contains("Failed to read row"),
            "{error:#}"
        );
        // The row's other columns can still be written.
        let writer = db.writer().await.unwrap();
        writer
            .conn
            .execute(
                "UPDATE node_image SET memory = 2048 WHERE model = 'future_model'",
                (),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn no_enum_rule_triggers_are_created_and_older_ones_are_dropped() {
        let db = open_in_memory().await.unwrap();
        apply_schema(&db).await.unwrap();
        assert_eq!(trigger_count(&db, "%_rules_%").await, 0);
        assert_eq!(trigger_count(&db, "user_protect_admin_%").await, 3);

        // A database from before the triggers were removed, which still has them.
        {
            let writer = db.writer().await.unwrap();
            writer
                .conn
                .execute_batch(
                    "CREATE TRIGGER node_image_rules_insert BEFORE INSERT ON node_image \
                     BEGIN SELECT 1; END; PRAGMA user_version = 7;",
                )
                .await
                .unwrap();
        }
        apply_schema(&db).await.unwrap();
        assert_eq!(trigger_count(&db, "%_rules_%").await, 0);
        assert_eq!(version(&db).await, SCHEMA_VERSION);
    }

    #[tokio::test]
    async fn a_migration_that_fails_halfway_leaves_the_database_as_it_was() {
        let db = open_in_memory().await.unwrap();
        apply_schema(&db).await.unwrap();
        {
            let mut writer = db.writer().await.unwrap();
            let tx = writer.conn.transaction().await.unwrap();
            tx.execute("ALTER TABLE lab ADD COLUMN half_applied TEXT", ())
                .await
                .unwrap();
            tx.execute(&format!("PRAGMA user_version = {}", SCHEMA_VERSION + 1), ())
                .await
                .unwrap();
            assert!(
                tx.execute("ALTER TABLE missing ADD COLUMN x TEXT", ())
                    .await
                    .is_err()
            );
            // Dropped without a commit, as apply_schema's transaction is on an error.
        }

        assert_eq!(version(&db).await, SCHEMA_VERSION);
        let conn = db.reader().await.unwrap();
        let columns = query_count(
            &conn,
            "SELECT count(*) FROM pragma_table_info('lab') WHERE name = 'half_applied'",
            (),
        )
        .await
        .unwrap();
        assert_eq!(columns, 0);
    }

    #[tokio::test]
    async fn a_model_has_one_default_image_per_kind() {
        let db = open_in_memory().await.unwrap();
        apply_schema(&db).await.unwrap();
        let mut first = NodeConfig::get_model(NodeModel::UbuntuLinux);
        first.default = true;
        upsert_node_image(&db, first).await.unwrap();
        let mut second = NodeConfig::get_model(NodeModel::UbuntuLinux);
        second.version = "second".to_owned();
        second.default = false;
        upsert_node_image(&db, second).await.unwrap();

        // Below the db functions, which clear the old default first.
        let writer = db.writer().await.unwrap();
        let both = writer
            .conn
            .execute(
                "UPDATE node_image SET \"default\" = 1 WHERE model = 'ubuntu_linux' \
                 AND version = 'second'",
                (),
            )
            .await;
        assert!(both.is_err(), "two default images were stored");
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
    async fn apply_schema_corrects_data_interface_counts_of_older_databases() {
        // A version 1 database: the tables alone.
        let db = open_in_memory().await.unwrap();
        let writer = db.writer().await.unwrap();
        for batch in TABLES {
            writer.conn.execute_batch(batch).await.unwrap();
        }
        writer
            .conn
            .execute("PRAGMA user_version = 1", ())
            .await
            .unwrap();
        drop(writer);
        for model in [
            NodeModel::CiscoCsr1000v,
            NodeModel::CiscoIosvl2,
            NodeModel::UbuntuLinux,
        ] {
            let mut image = NodeConfig::get_model(model);
            image.data_interface_count = 16;
            upsert_node_image(&db, image).await.unwrap();
        }

        apply_schema(&db).await.unwrap();
        assert_eq!(version(&db).await, SCHEMA_VERSION);

        let counts: Vec<(NodeModel, u8)> = list_node_images(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|image| (image.model, image.data_interface_count))
            .collect();
        assert!(
            counts.contains(&(NodeModel::CiscoCsr1000v, 15)),
            "{counts:?}"
        );
        assert!(counts.contains(&(NodeModel::CiscoIosvl2, 15)), "{counts:?}");
        assert!(counts.contains(&(NodeModel::UbuntuLinux, 16)), "{counts:?}");
    }

    #[tokio::test]
    async fn apply_schema_corrects_ztp_image_defaults() {
        // A version 3 database, before the ISE correction.
        let db = open_in_memory().await.unwrap();
        let writer = db.writer().await.unwrap();
        for batch in TABLES.iter().chain(&migrations::ALL[..2]) {
            writer.conn.execute_batch(batch).await.unwrap();
        }
        writer
            .conn
            .execute("PRAGMA user_version = 3", ())
            .await
            .unwrap();
        drop(writer);
        for model in [NodeModel::CiscoIse, NodeModel::CiscoAsav] {
            let mut image = NodeConfig::get_model(model);
            image.ztp_method = ZtpMethod::Cdrom;
            upsert_node_image(&db, image).await.unwrap();
        }
        for model in [NodeModel::GenericVm, NodeModel::CiscoIosv] {
            let mut image = NodeConfig::get_model(model);
            image.cdrom_bus = DiskBuses::Virtio;
            upsert_node_image(&db, image).await.unwrap();
        }

        apply_schema(&db).await.unwrap();
        assert_eq!(version(&db).await, SCHEMA_VERSION);

        let methods: Vec<(NodeModel, ZtpMethod)> = list_node_images(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|image| (image.model, image.ztp_method))
            .collect();
        assert!(
            methods.contains(&(NodeModel::CiscoIse, ZtpMethod::Disk)),
            "{methods:?}"
        );
        assert!(
            methods.contains(&(NodeModel::CiscoAsav, ZtpMethod::Cdrom)),
            "{methods:?}"
        );

        let buses: Vec<(NodeModel, DiskBuses)> = list_node_images(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|image| (image.model, image.cdrom_bus))
            .collect();
        assert!(
            buses.contains(&(NodeModel::GenericVm, DiskBuses::Sata)),
            "{buses:?}"
        );
        assert!(
            buses.contains(&(NodeModel::CiscoIosv, DiskBuses::Virtio)),
            "{buses:?}"
        );
    }

    #[tokio::test]
    async fn apply_schema_moves_vsrx_images_to_the_cdrom_ztp_method() {
        // A version 4 database, before the vSRX correction.
        let db = open_in_memory().await.unwrap();
        let writer = db.writer().await.unwrap();
        for batch in TABLES.iter().chain(&migrations::ALL[..3]) {
            writer.conn.execute_batch(batch).await.unwrap();
        }
        writer
            .conn
            .execute("PRAGMA user_version = 4", ())
            .await
            .unwrap();
        drop(writer);
        for model in [NodeModel::JuniperVsrxv3, NodeModel::AristaVeos] {
            let mut image = NodeConfig::get_model(model);
            image.ztp_method = ZtpMethod::Tftp;
            upsert_node_image(&db, image).await.unwrap();
        }

        apply_schema(&db).await.unwrap();
        assert_eq!(version(&db).await, SCHEMA_VERSION);

        let methods: Vec<(NodeModel, ZtpMethod)> = list_node_images(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|image| (image.model, image.ztp_method))
            .collect();
        assert!(
            methods.contains(&(NodeModel::JuniperVsrxv3, ZtpMethod::Cdrom)),
            "{methods:?}"
        );
        assert!(
            methods.contains(&(NodeModel::AristaVeos, ZtpMethod::Tftp)),
            "{methods:?}"
        );
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
    async fn apply_schema_creates_the_protection_triggers() {
        let db = open_in_memory().await.unwrap();
        apply_schema(&db).await.unwrap();
        // Three admin protection triggers, and the two that keep interfaces used once.
        assert_eq!(trigger_count(&db, "%").await, 3 + 2);
    }
}
