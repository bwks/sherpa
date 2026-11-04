//! Schema versions and their application
//!
//! The database's `user_version` is its schema version. Version 1 is the tables in `sql/schema/`;
//! each file in `sql/migrations/` takes the schema up one version, in order. A released version
//! is never changed, and a later change to the schema is a new migration file. The rule triggers
//! derived from the Rust enums are not versioned: they are dropped and created again on every
//! application.

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
    use shared::data::{DiskBuses, NodeConfig, NodeModel, ZtpMethod};

    use super::*;
    use crate::{create_node_image, list_node_images, open_in_memory};

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
            create_node_image(&db, image).await.unwrap();
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
            create_node_image(&db, image).await.unwrap();
        }
        for model in [NodeModel::GenericVm, NodeModel::CiscoIosv] {
            let mut image = NodeConfig::get_model(model);
            image.cdrom_bus = DiskBuses::Virtio;
            create_node_image(&db, image).await.unwrap();
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
            create_node_image(&db, image).await.unwrap();
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
