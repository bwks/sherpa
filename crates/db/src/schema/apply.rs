//! Schema versions and their application
//!
//! The database's `user_version` is its schema version. Version 1 is the tables in `sql/schema/`;
//! each file in `sql/migrations/` takes the schema up one version, in order. A released version
//! is never changed, and a later change to the schema is a new migration file.

use anyhow::{Context, Result, bail};
use tracing::instrument;

use super::{bridge, lab, link, migrations, node, node_image, server_key, user};
use crate::Database;
use crate::sql::{Query, query_count};

/// Schema version 1: every table, parents before children.
const TABLES: &[&str] = &[
    user::TABLES,
    node_image::TABLES,
    lab::TABLES,
    node::TABLES,
    link::TABLES,
    bridge::TABLES,
];

/// The database's schema version. It takes no parameters, so it is not a query file.
const USER_VERSION: Query<0> = Query::new("PRAGMA user_version");

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
/// - If a lab still uses the server's shared SSH key when the migration that drops
///   `lab.ssh_key` is next; each one is named, and nothing is migrated
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

    let version = query_count(&tx, USER_VERSION.bind([]))
        .await
        .context("Failed to read the database schema version")?;
    refuse_newer_version(version)?;

    for target in (version + 1)..=SCHEMA_VERSION {
        tracing::info!(from = target - 1, to = target, "Migrating database schema");
        let batches = match target {
            1 => TABLES,
            _ => migrations::ALL
                .get(target - 2..target - 1)
                .context(format!("Schema version {target} has no migration"))?,
        };
        for batch in batches {
            if *batch == migrations::DROP_LAB_SSH_KEY {
                server_key::refuse_server_key_labs(&server_key::server_key_labs(&tx).await?)?;
            }
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

/// Check that `apply_schema` can bring the database to this build's schema version, without
/// changing it. It makes the same refusals.
///
/// # Errors
///
/// - If the database has a newer schema version than this build knows
/// - If a lab still uses the server's shared SSH key, which this build removes; each one is named
/// - If the database cannot be read
#[instrument(skip(db), level = "debug")]
pub async fn check_schema_upgrade(db: &Database) -> Result<()> {
    let conn = db.reader().await?;
    let version = query_count(&conn, USER_VERSION.bind([]))
        .await
        .context("Failed to read the database schema version")?;
    refuse_newer_version(version)?;
    server_key::refuse_server_key_labs(&server_key::server_key_labs(&conn).await?)
}

/// Refuse a database whose schema `version` is newer than this build knows, for example after a
/// downgrade of Sherpa.
fn refuse_newer_version(version: usize) -> Result<()> {
    if version > SCHEMA_VERSION {
        bail!(
            "The database has schema version {version}, newer than {SCHEMA_VERSION} that this \
             Sherpa build supports"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use shared::data::{
        BiosTypes, BootDiskSettings, CdromSettings, DiskBuses, LabPhase, MachineSettings,
        MachineType, NodeConfig, NodeKind, NodeModel, NodeState, Ztp,
    };
    use shared::validators::check_ztp;
    use turso::Value;

    use super::*;
    use crate::persistence::{new_key, node_image_values};
    use crate::{
        DefaultRequest, find_lab, list_node_images, list_nodes_by_lab, open_in_memory,
        upsert_node_image,
    };

    /// The eleven ZTP methods a release before the one `ztp` column could store.
    const OLD_ZTP_METHODS: [&str; 11] = [
        "cloud-init",
        "cdrom",
        "disk",
        "http",
        "ignition",
        "ipxe",
        "nocloud",
        "tftp",
        "usb",
        "volume",
        "none",
    ];

    async fn version(db: &Database) -> usize {
        let conn = db.reader().await.unwrap();
        query_count(&conn, USER_VERSION.bind([])).await.unwrap()
    }

    /// Insert `image` into a database at a version before 16, whose `node_image` table has the
    /// `ztp_enable` and `ztp_method` columns, and before version 9 the
    /// `dedicated_management_interface` column too, none of which today's queries write. The
    /// two ZTP values are stored as given, as an older release stored them. Returns the row's
    /// key.
    async fn insert_into_older_schema(
        db: &Database,
        image: &NodeConfig,
        ztp_enable: bool,
        ztp_method: &str,
    ) -> String {
        let key = new_key();
        let mut values = Vec::from(node_image_values(image, &key).unwrap());
        // Today's last column is `ztp`, which the older table does not have.
        values.pop();
        values.push(Value::from(ztp_enable));
        values.push(Value::from(ztp_method));
        let mut columns = "id, model, version, repo, os_variant, kind, bios, cpu_count, \
             cpu_architecture, cpu_model, machine_type, vmx_enabled, memory, hdd_bus, cdrom, \
             cdrom_bus, ztp_username, ztp_password, ztp_password_auth, data_interface_count, \
             interface_prefix, interface_type, interface_mtu, first_interface_index, \
             management_interface, reserved_interface_count, \"default\", boot_mode, ztp_enable, \
             ztp_method"
            .to_owned();
        let writer = db.writer().await.unwrap();
        let dedicated = query_count(
            &writer.conn,
            Query::new(
                "SELECT count(*) FROM pragma_table_info('node_image') \
             WHERE name = 'dedicated_management_interface'",
            )
            .bind([]),
        )
        .await
        .unwrap();
        if dedicated == 1 {
            columns.push_str(", dedicated_management_interface");
            values.push(Value::from(false));
        }
        let placeholders = (1..=values.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        writer
            .conn
            .execute(
                &format!("INSERT INTO node_image ({columns}) VALUES ({placeholders})"),
                values,
            )
            .await
            .unwrap();
        key
    }

    /// A database with the tables and the first `migrations` migrations applied.
    async fn database_at(migrations: usize) -> Database {
        let db = open_in_memory().await.unwrap();
        {
            let writer = db.writer().await.unwrap();
            for batch in TABLES.iter().chain(&migrations::ALL[..migrations]) {
                writer.conn.execute_batch(batch).await.unwrap();
            }
            writer
                .conn
                .execute(&format!("PRAGMA user_version = {}", migrations + 1), ())
                .await
                .unwrap();
        }
        db
    }

    /// What the migration to the one `ztp` column makes of a stored `(enable, method)` pair.
    /// The models a database from before the one `ztp` setting can hold: every model but those
    /// added since, which migration `0016` does not name.
    fn models_before_the_ztp_setting() -> Vec<NodeModel> {
        NodeModel::to_vec()
            .into_iter()
            .filter(|model| !matches!(model, NodeModel::CiscoIol | NodeModel::CiscoIoll2))
            .collect()
    }

    fn migrated_ztp(model: NodeModel, enable: bool, method: &str) -> Ztp {
        let image = NodeConfig::get_model(model);
        if !enable {
            return Ztp::Disabled;
        }
        if image.kind != NodeKind::VirtualMachine {
            // The four models with mounted configs are the only ones whose default is not off.
            return image.ztp.method;
        }
        let stored: Option<Ztp> =
            serde_json::from_value(serde_json::Value::String(method.to_owned())).ok();
        match stored {
            Some(ztp) if model.ztp_methods().contains(&ztp) => ztp,
            // `volume` on a VM, `none` and `ipxe`: their arms generated nothing.
            Some(Ztp::Volume) | None => Ztp::Disabled,
            Some(_) => image.ztp.method,
        }
    }

    async fn trigger_count(db: &Database, pattern: &str) -> usize {
        let conn = db.reader().await.unwrap();
        query_count(
            &conn,
            Query::new(
                "SELECT count(*) FROM sqlite_schema WHERE type = 'trigger' AND name LIKE ?1",
            )
            .bind([pattern.into()]),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_value_a_newer_release_stored_is_kept_and_named_when_read() {
        let db = open_in_memory().await.unwrap();
        apply_schema(&db).await.unwrap();
        upsert_node_image(
            &db,
            NodeConfig::get_model(NodeModel::UbuntuLinux),
            DefaultRequest::Keep,
        )
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
        let db = open_in_memory().await.unwrap();
        {
            let writer = db.writer().await.unwrap();
            for batch in TABLES.iter().chain(&migrations::ALL[..6]) {
                writer.conn.execute_batch(batch).await.unwrap();
            }
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
            Query::new("SELECT count(*) FROM pragma_table_info('lab') WHERE name = 'half_applied'")
                .bind([]),
        )
        .await
        .unwrap();
        assert_eq!(columns, 0);
    }

    #[tokio::test]
    async fn a_model_has_one_default_image_per_kind() {
        let db = open_in_memory().await.unwrap();
        apply_schema(&db).await.unwrap();
        let first = NodeConfig::get_model(NodeModel::UbuntuLinux);
        upsert_node_image(&db, first, DefaultRequest::Make)
            .await
            .unwrap();
        let mut second = NodeConfig::get_model(NodeModel::UbuntuLinux);
        second.version = "second".to_owned();
        upsert_node_image(&db, second, DefaultRequest::Keep)
            .await
            .unwrap();

        // Below the db functions, which clear the old default first.
        let writer = db.writer().await.unwrap();
        let both = writer
            .conn
            .execute(
                r#"
                UPDATE node_image SET "default" = 1
                WHERE model = 'ubuntu_linux' AND version = 'second'
                "#,
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
            image.interfaces.data_count = 16;
            insert_into_older_schema(&db, &image, true, "cloud-init").await;
        }

        apply_schema(&db).await.unwrap();
        assert_eq!(version(&db).await, SCHEMA_VERSION);

        let counts: Vec<(NodeModel, u8)> = list_node_images(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|image| (image.model, image.interfaces.data_count))
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
            insert_into_older_schema(&db, &NodeConfig::get_model(model), true, "cdrom").await;
        }
        for model in [NodeModel::GenericVm, NodeModel::CiscoIosv] {
            let mut image = NodeConfig::get_model(model);
            image.cdrom.bus = DiskBuses::Virtio;
            insert_into_older_schema(&db, &image, true, "cloud-init").await;
        }

        apply_schema(&db).await.unwrap();
        assert_eq!(version(&db).await, SCHEMA_VERSION);

        let methods: Vec<(NodeModel, Ztp)> = list_node_images(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|image| (image.model, image.ztp.method))
            .collect();
        assert!(
            methods.contains(&(NodeModel::CiscoIse, Ztp::Disk)),
            "{methods:?}"
        );
        assert!(
            methods.contains(&(NodeModel::CiscoAsav, Ztp::Cdrom)),
            "{methods:?}"
        );

        let buses: Vec<(NodeModel, DiskBuses)> = list_node_images(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|image| (image.model, image.cdrom.bus))
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
            insert_into_older_schema(&db, &NodeConfig::get_model(model), true, "tftp").await;
        }

        apply_schema(&db).await.unwrap();
        assert_eq!(version(&db).await, SCHEMA_VERSION);

        let methods: Vec<(NodeModel, Ztp)> = list_node_images(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|image| (image.model, image.ztp.method))
            .collect();
        assert!(
            methods.contains(&(NodeModel::JuniperVsrxv3, Ztp::Cdrom)),
            "{methods:?}"
        );
        assert!(
            methods.contains(&(NodeModel::AristaVeos, Ztp::Tftp)),
            "{methods:?}"
        );
    }

    #[tokio::test]
    async fn apply_schema_drops_the_dedicated_management_interface_column() {
        let db = open_in_memory().await.unwrap();
        {
            let writer = db.writer().await.unwrap();
            for batch in TABLES.iter().chain(&migrations::ALL[..7]) {
                writer.conn.execute_batch(batch).await.unwrap();
            }
            writer
                .conn
                .execute("PRAGMA user_version = 8", ())
                .await
                .unwrap();
        }
        insert_into_older_schema(
            &db,
            &NodeConfig::get_model(NodeModel::UbuntuLinux),
            true,
            "cloud-init",
        )
        .await;

        apply_schema(&db).await.unwrap();

        let conn = db.reader().await.unwrap();
        let columns = query_count(
            &conn,
            Query::new(
                "SELECT COUNT(*) FROM pragma_table_info('node_image') \
             WHERE name = 'dedicated_management_interface'",
            )
            .bind([]),
        )
        .await
        .unwrap();
        assert_eq!(columns, 0);
        assert_eq!(list_node_images(&db).await.unwrap().len(), 1);
    }

    /// User `alice`, the owner of the old labs.
    const OLD_USER: &str = "INSERT INTO user (id, username, password_hash, is_admin, ssh_keys,
             created_at, updated_at)
         VALUES ('u1', 'alice', 'x', 0, '[]', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');";

    /// `alice`'s lab `lab-0001`, named `Old`, in a database from before lab keys, which still
    /// stored the lab's status. The `ssh_key` column gives it the server's key.
    const LAB_BEFORE_LAB_KEYS: &str = "INSERT INTO lab (id, lab_id, name, user,
             loopback_network, management_network, gateway_ipv4, router_ipv4, status)
         VALUES ('l1', 'lab-0001', 'Old', 'u1', '127.127.1.0/24', '172.31.1.0/24',
             '172.31.1.1', '172.31.1.2', 'running');";

    /// The number of migrations before the one that drops `lab.ssh_key`.
    fn migrations_before_lab_key_drop() -> usize {
        migrations::ALL
            .iter()
            .position(|batch| *batch == migrations::DROP_LAB_SSH_KEY)
            .unwrap()
    }

    /// A database at the version before `lab.ssh_key` was dropped, with `alice`'s lab
    /// `lab-0001`, named `Old`, recording `ssh_key`.
    async fn database_before_lab_keys_only(ssh_key: &str) -> Database {
        let db = database_at(migrations_before_lab_key_drop()).await;
        {
            let writer = db.writer().await.unwrap();
            writer.conn.execute_batch(OLD_USER).await.unwrap();
            writer
                .conn
                .execute(
                    "INSERT INTO lab (id, lab_id, name, user, loopback_network,
                         management_network, gateway_ipv4, router_ipv4, ssh_key)
                     VALUES ('l1', 'lab-0001', 'Old', 'u1', '127.127.1.0/24', '172.31.1.0/24',
                         '172.31.1.1', '172.31.1.2', ?1)",
                    [ssh_key],
                )
                .await
                .unwrap();
        }
        db
    }

    async fn has_ssh_key_column(db: &Database) -> bool {
        let conn = db.reader().await.unwrap();
        query_count(
            &conn,
            Query::new("SELECT count(*) FROM pragma_table_info('lab') WHERE name = 'ssh_key'")
                .bind([]),
        )
        .await
        .unwrap()
            == 1
    }

    #[tokio::test]
    async fn a_lab_created_before_lab_keys_is_refused_and_named() {
        // A database from before lab keys: its lab gets the server's key when migrated.
        let db = database_at(9).await;
        {
            let writer = db.writer().await.unwrap();
            writer.conn.execute_batch(OLD_USER).await.unwrap();
            writer
                .conn
                .execute_batch(LAB_BEFORE_LAB_KEYS)
                .await
                .unwrap();
        }

        let error = format!("{:#}", apply_schema(&db).await.unwrap_err());

        assert!(error.contains("lab-0001 (Old, owned by alice)"), "{error}");
        assert!(error.contains("server's shared SSH key"), "{error}");
        // Not one migration of the run is kept.
        assert_eq!(version(&db).await, 10);
    }

    #[tokio::test]
    async fn a_lab_on_the_server_key_stops_the_column_from_being_dropped() {
        let db = database_before_lab_keys_only("server").await;

        let error = format!("{:#}", apply_schema(&db).await.unwrap_err());

        assert!(error.contains("lab-0001 (Old, owned by alice)"), "{error}");
        assert_eq!(version(&db).await, migrations_before_lab_key_drop() + 1);
        assert!(has_ssh_key_column(&db).await);
    }

    #[tokio::test]
    async fn the_ssh_key_column_is_dropped_once_every_lab_has_its_own_key() {
        let db = database_before_lab_keys_only("lab").await;

        apply_schema(&db).await.unwrap();

        assert_eq!(version(&db).await, SCHEMA_VERSION);
        assert!(!has_ssh_key_column(&db).await);
        assert_eq!(
            find_lab(&db, "lab-0001").await.unwrap().unwrap().name,
            "Old"
        );
    }

    #[tokio::test]
    async fn the_upgrade_check_refuses_without_changing_the_database() {
        let db = database_before_lab_keys_only("server").await;

        let error = format!("{:#}", check_schema_upgrade(&db).await.unwrap_err());

        assert!(error.contains("lab-0001 (Old, owned by alice)"), "{error}");
        assert_eq!(version(&db).await, migrations_before_lab_key_drop() + 1);
        assert!(has_ssh_key_column(&db).await);
    }

    #[tokio::test]
    async fn the_upgrade_check_passes_a_database_this_release_can_use() {
        let fresh = open_in_memory().await.unwrap();
        check_schema_upgrade(&fresh).await.unwrap();
        assert_eq!(version(&fresh).await, 0);

        let own_keys = database_before_lab_keys_only("lab").await;
        check_schema_upgrade(&own_keys).await.unwrap();
        assert_eq!(
            version(&own_keys).await,
            migrations_before_lab_key_drop() + 1
        );

        let current = open_in_memory().await.unwrap();
        apply_schema(&current).await.unwrap();
        check_schema_upgrade(&current).await.unwrap();
    }

    #[tokio::test]
    async fn the_upgrade_check_refuses_a_newer_schema() {
        let db = database_at(migrations::ALL.len()).await;
        {
            let writer = db.writer().await.unwrap();
            writer
                .conn
                .execute(&format!("PRAGMA user_version = {}", SCHEMA_VERSION + 1), ())
                .await
                .unwrap();
        }

        let error = format!("{:#}", check_schema_upgrade(&db).await.unwrap_err());

        assert!(error.contains("newer than"), "{error}");
    }

    /// A database at the version before the node lifecycle was explicit, with one lab and a
    /// node of each recorded state.
    async fn database_before_the_node_lifecycle(states: &[&str]) -> Database {
        let db = open_in_memory().await.unwrap();
        {
            let writer = db.writer().await.unwrap();
            for batch in TABLES.iter().chain(&migrations::ALL[..11]) {
                writer.conn.execute_batch(batch).await.unwrap();
            }
            writer
                .conn
                .execute_batch(
                    "PRAGMA user_version = 12;
                     INSERT INTO user (id, username, password_hash, is_admin, ssh_keys,
                         created_at, updated_at)
                     VALUES ('u1', 'alice', 'x', 0, '[]', '2026-01-01T00:00:00Z',
                         '2026-01-01T00:00:00Z');
                     INSERT INTO lab (id, lab_id, name, user, loopback_network,
                         management_network, gateway_ipv4, router_ipv4, status, ssh_key)
                     VALUES ('l1', 'lab-0001', 'Old', 'u1', '127.127.1.0/24', '172.31.1.0/24',
                         '172.31.1.1', '172.31.1.2', 'running', 'lab');",
                )
                .await
                .unwrap();
        }
        let image_key = insert_into_older_schema(
            &db,
            &NodeConfig::get_model(NodeModel::UbuntuLinux),
            true,
            "cloud-init",
        )
        .await;
        let writer = db.writer().await.unwrap();
        for (index, state) in (0_i64..).zip(states) {
            writer
                .conn
                .execute(
                    "INSERT INTO node (id, name, \"index\", image, lab, state) \
                     VALUES (?1, ?1, ?2, ?3, 'l1', ?4)",
                    (format!("n{index}"), index, image_key.as_str(), *state),
                )
                .await
                .unwrap();
        }
        drop(writer);
        db
    }

    #[tokio::test]
    async fn nodes_recorded_created_are_stopped_after_the_upgrade() {
        // `created` was the scanner's word for a container Docker had created and not started.
        let db = database_before_the_node_lifecycle(&["created", "running", "unknown"]).await;

        apply_schema(&db).await.unwrap();

        let lab = find_lab(&db, "lab-0001").await.unwrap().unwrap();
        let mut states: Vec<NodeState> = list_nodes_by_lab(&db, lab.id.unwrap())
            .await
            .unwrap()
            .into_iter()
            .map(|node| node.state)
            .collect();
        states.sort_by_key(ToString::to_string);
        assert_eq!(
            states,
            [NodeState::Running, NodeState::Stopped, NodeState::Unknown]
        );
    }

    #[tokio::test]
    async fn a_node_state_this_build_does_not_know_does_not_fail_the_listing() {
        // M1.8: a state written by a newer build is read as unknown.
        let db = database_before_the_node_lifecycle(&["hibernating", "running"]).await;
        apply_schema(&db).await.unwrap();

        let lab = find_lab(&db, "lab-0001").await.unwrap().unwrap();
        let nodes = list_nodes_by_lab(&db, lab.id.unwrap()).await.unwrap();
        assert_eq!(nodes.len(), 2);
        assert!(nodes.iter().any(|node| node.state == NodeState::Unknown));
    }

    #[tokio::test]
    async fn models_left_without_a_default_image_get_their_newest() {
        let db = open_in_memory().await.unwrap();
        {
            let writer = db.writer().await.unwrap();
            for batch in TABLES.iter().chain(&migrations::ALL[..12]) {
                writer.conn.execute_batch(batch).await.unwrap();
            }
            writer
                .conn
                .execute("PRAGMA user_version = 13", ())
                .await
                .unwrap();
        }
        // As an earlier release could leave it: Ubuntu has images and no default.
        for (model, version, default) in [
            (NodeModel::UbuntuLinux, "old", false),
            (NodeModel::UbuntuLinux, "new", false),
            (NodeModel::FedoraLinux, "kept", true),
            (NodeModel::FedoraLinux, "other", false),
        ] {
            let mut image = NodeConfig::get_model(model);
            version.clone_into(&mut image.version);
            image.default = default;
            insert_into_older_schema(&db, &image, true, "cloud-init").await;
        }

        apply_schema(&db).await.unwrap();

        let defaults: Vec<(NodeModel, String)> = list_node_images(&db)
            .await
            .unwrap()
            .into_iter()
            .filter(|image| image.default)
            .map(|image| (image.model, image.version))
            .collect();
        assert_eq!(defaults.len(), 2, "{defaults:?}");
        assert!(defaults.contains(&(NodeModel::UbuntuLinux, "new".to_owned())));
        assert!(defaults.contains(&(NodeModel::FedoraLinux, "kept".to_owned())));
    }

    #[tokio::test]
    async fn labs_from_before_the_lab_lifecycle_are_ready_and_their_status_is_not_stored() {
        // Every lab that exists before the phase was stored had been built.
        let db = open_in_memory().await.unwrap();
        {
            let writer = db.writer().await.unwrap();
            for batch in TABLES.iter().chain(&migrations::ALL[..13]) {
                writer.conn.execute_batch(batch).await.unwrap();
            }
            writer
                .conn
                .execute_batch(
                    "PRAGMA user_version = 14;
                     INSERT INTO user (id, username, password_hash, is_admin, ssh_keys,
                         created_at, updated_at)
                     VALUES ('u1', 'alice', 'x', 0, '[]', '2026-01-01T00:00:00Z',
                         '2026-01-01T00:00:00Z');
                     INSERT INTO lab (id, lab_id, name, user, loopback_network,
                         management_network, gateway_ipv4, router_ipv4, status, ssh_key)
                     VALUES ('l1', 'lab-0001', 'Old', 'u1', '127.127.1.0/24', '172.31.1.0/24',
                         '172.31.1.1', '172.31.1.2', 'stopped', 'lab');",
                )
                .await
                .unwrap();
        }

        apply_schema(&db).await.unwrap();

        let lab = find_lab(&db, "lab-0001").await.unwrap().unwrap();
        assert_eq!(lab.phase, LabPhase::Ready);
        let writer = db.writer().await.unwrap();
        assert!(
            writer
                .conn
                .execute("SELECT status FROM lab", ())
                .await
                .is_err()
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

    #[tokio::test]
    async fn apply_schema_folds_every_ztp_pair_into_the_one_setting() {
        // A version 15 database, before the one `ztp` column.
        let db = database_at(14).await;
        for model in models_before_the_ztp_setting() {
            for enable in [false, true] {
                for method in OLD_ZTP_METHODS {
                    let mut image = NodeConfig::get_model(model);
                    image.version = format!("{enable}-{method}");
                    image.default = false;
                    insert_into_older_schema(&db, &image, enable, method).await;
                }
            }
        }

        apply_schema(&db).await.unwrap();
        assert_eq!(version(&db).await, SCHEMA_VERSION);

        let images = list_node_images(&db).await.unwrap();
        assert_eq!(images.len(), models_before_the_ztp_setting().len() * 22);
        for image in images {
            let (enable, method) = image.version.split_once('-').unwrap();
            let expected = migrated_ztp(image.model, enable == "true", method);
            assert_eq!(
                image.ztp.method, expected,
                "{} stored as {}",
                image.model, image.version
            );
            check_ztp(image.model, image.ztp.method).unwrap();
        }
    }

    #[tokio::test]
    async fn apply_schema_keeps_the_settings_this_release_ships_with() {
        // The pairs the model defaults of the release before stored: enabled with the model's
        // method, `none` for every container, and off for unikernels.
        let db = database_at(14).await;
        for model in models_before_the_ztp_setting() {
            let mut image = NodeConfig::get_model(model);
            image.default = false;
            let (enable, method) = match image.kind {
                NodeKind::VirtualMachine => (true, image.ztp.method.to_string()),
                NodeKind::Container => (true, "none".to_owned()),
                NodeKind::Unikernel => (false, "none".to_owned()),
            };
            insert_into_older_schema(&db, &image, enable, &method).await;
        }

        apply_schema(&db).await.unwrap();

        for image in list_node_images(&db).await.unwrap() {
            let default = NodeConfig::get_model(image.model).ztp.method;
            assert_eq!(image.ztp.method, default, "{}", image.model);
        }
    }

    #[tokio::test]
    async fn apply_schema_turns_ztp_off_for_a_model_this_build_does_not_know() {
        let db = database_at(14).await;
        let key = insert_into_older_schema(
            &db,
            &NodeConfig::get_model(NodeModel::UbuntuLinux),
            true,
            "cloud-init",
        )
        .await;
        {
            let writer = db.writer().await.unwrap();
            writer
                .conn
                .execute("UPDATE node_image SET model = 'future_model'", ())
                .await
                .unwrap();
        }

        apply_schema(&db).await.unwrap();

        let conn = db.reader().await.unwrap();
        let disabled = query_count(
            &conn,
            Query::new("SELECT count(*) FROM node_image WHERE id = ?1 AND ztp = 'disabled'")
                .bind([key.as_str().into()]),
        )
        .await
        .unwrap();
        assert_eq!(disabled, 1);
        let old_columns = query_count(
            &conn,
            Query::new(
                "SELECT count(*) FROM pragma_table_info('node_image') \
             WHERE name IN ('ztp_enable', 'ztp_method')",
            )
            .bind([]),
        )
        .await
        .unwrap();
        assert_eq!(old_columns, 0);
    }

    #[tokio::test]
    async fn apply_schema_resets_the_settings_a_kind_does_not_use_and_keeps_the_rest() {
        // A version 17 database. Every model is stored twice: as it ships, and edited to
        // settings other than the defaults, as the web form or an older release could store.
        let db = database_at(16).await;
        let mut stored = Vec::new();
        for model in NodeModel::to_vec() {
            let shipped = NodeConfig::get_model(model);
            let edited = NodeConfig {
                version: "edited".to_owned(),
                machine: MachineSettings {
                    kind: MachineType::Q35,
                    bios: BiosTypes::Uefi,
                },
                boot_disk: BootDiskSettings {
                    bus: DiskBuses::Sata,
                },
                cdrom: CdromSettings {
                    image: Some("extra.iso".to_owned()),
                    bus: DiskBuses::Sata,
                },
                ..shipped.clone()
            };
            for (image, request) in [
                (shipped, DefaultRequest::Make),
                (edited, DefaultRequest::Keep),
            ] {
                stored.push(upsert_node_image(&db, image, request).await.unwrap());
            }
        }

        apply_schema(&db).await.unwrap();

        let read = list_node_images(&db).await.unwrap();
        assert_eq!(read.len(), stored.len());
        for before in stored {
            let mut expected = before.clone();
            if !expected.kind.uses_machine_type() {
                expected.machine.kind = MachineType::default();
            }
            if !expected.kind.uses_firmware_and_disks() {
                expected.machine.bios = BiosTypes::default();
                expected.boot_disk = BootDiskSettings::default();
                expected.cdrom = CdromSettings::default();
            }
            let after = read
                .iter()
                .find(|image| image.id == before.id)
                .unwrap_or_else(|| panic!("{} {} is gone", before.model, before.version));
            assert_eq!(
                serde_json::to_value(after).unwrap(),
                serde_json::to_value(&expected).unwrap(),
                "{} {}",
                before.model,
                before.version
            );
        }
    }
}
