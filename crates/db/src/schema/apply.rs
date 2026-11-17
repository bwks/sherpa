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
        BiosTypes, BootDiskSettings, CdromSettings, DbCpuPins, DiskBuses, LabPhase,
        MachineSettings, MachineType, ModelKind, NodeConfig, NodeCpuPins, NodeKind, NodeModel,
        NodeState, REMOVED_MODELS, ReadyCheck, RecordId, Ztp,
    };
    use shared::konst::{SSH_PORT, TALOS_API_PORT};
    use shared::validators::check_ztp;
    use turso::Value;

    use super::*;
    use crate::persistence::{key_of, new_key, node_image_values};
    use crate::{
        DefaultRequest, count_nodes_by_ssh_username, device_password_set, find_lab,
        find_node_image_by_id, get_user, list_cpu_pins, list_node_images, list_nodes_by_lab,
        open_in_memory, read_lab_definition, update_node_image, upsert_node_image,
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

    /// Today's `node_image` columns, in order, but the last, `ztp`.
    const IMAGE_COLUMNS_BEFORE_ZTP: &str = "id, model, version, repo, os_variant, kind, bios, \
         cpu_count, cpu_architecture, cpu_model, machine_type, vmx_enabled, memory, hdd_bus, \
         cdrom, cdrom_bus, data_interface_count, interface_prefix, interface_type, interface_mtu, \
         first_interface_index, management_interface, reserved_interface_count, \"default\", \
         boot_mode";

    /// The name `image`'s model was stored under before migration 31 renamed the models.
    fn name_before_the_renames(image: &NodeConfig) -> String {
        REMOVED_MODELS
            .iter()
            .find(|(_, model, kind)| *model == image.model && *kind == image.kind)
            .map_or_else(
                || image.model.to_string(),
                |(name, _, _)| (*name).to_owned(),
            )
    }

    /// Insert `image` into a database at a version before 16, whose `node_image` table has the
    /// `ztp_enable` and `ztp_method` columns, and before version 9 the
    /// `dedicated_management_interface` column too, none of which today's queries write. The
    /// two ZTP values are stored as given, as an older release stored them, and the ZTP login
    /// columns unset. Returns the row's key.
    async fn insert_into_older_schema(
        db: &Database,
        image: &NodeConfig,
        ztp_enable: bool,
        ztp_method: &str,
    ) -> String {
        let key = new_key();
        let mut values = Vec::from(node_image_values(image, &key).unwrap());
        values[1] = Value::from(name_before_the_renames(image));
        // Today's last column is `ztp`, which the older table does not have.
        values.pop();
        // The ZTP login columns, which today's table does not have, unset.
        values.extend([Value::Null, Value::Null, Value::from(false)]);
        values.push(Value::from(ztp_enable));
        values.push(Value::from(ztp_method));
        let mut columns = format!(
            "{IMAGE_COLUMNS_BEFORE_ZTP}, ztp_username, ztp_password, ztp_password_auth, \
             ztp_enable, ztp_method"
        );
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

    /// Insert `image` into a database from before migration 26, whose `node_image` table still
    /// has the ZTP login columns `ztp_username`, `ztp_password` and `ztp_password_auth`, which
    /// today's queries do not write. They are stored as `login` gives them: the username, the
    /// stored password text and the password-auth flag. `image.default` is stored as it is.
    /// Returns the image with its id.
    async fn insert_with_ztp_login(
        db: &Database,
        image: &NodeConfig,
        login: (Option<&str>, Option<&str>, bool),
    ) -> NodeConfig {
        let key = new_key();
        let mut values = Vec::from(node_image_values(image, &key).unwrap());
        values[1] = Value::from(name_before_the_renames(image));
        let (username, password, password_auth) = login;
        values.extend([
            Value::from(username),
            Value::from(password),
            Value::from(password_auth),
        ]);
        let placeholders = (1..=values.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        {
            let writer = db.writer().await.unwrap();
            writer
                .conn
                .execute(
                    &format!(
                        "INSERT INTO node_image ({IMAGE_COLUMNS_BEFORE_ZTP}, ztp, ztp_username, \
                         ztp_password, ztp_password_auth) VALUES ({placeholders})"
                    ),
                    values,
                )
                .await
                .unwrap();
        }
        // Read back as it is stored, but under its model's name today: the row keeps the name
        // it had before the renames until the migrations run.
        NodeConfig {
            id: Some(RecordId::new("node_image", key)),
            ..image.clone()
        }
    }

    /// The shipped image of `model`, as its default, stored with no ZTP login in a database from
    /// before migration 26. Returns the image's key.
    async fn insert_default_before_the_login_drop(db: &Database, model: NodeModel) -> String {
        let image = NodeConfig {
            default: true,
            ..ModelKind::default_for(model).node_config()
        };
        let stored = insert_with_ztp_login(db, &image, (None, None, false)).await;
        key_of(&stored.id.unwrap(), "node_image").unwrap()
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
        let image = ModelKind::default_for(model).node_config();
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
            Some(ztp) if ModelKind::default_for(model).ztp_methods().contains(&ztp) => ztp,
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
            ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
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
        let first = ModelKind::default_for(NodeModel::UbuntuLinux).node_config();
        upsert_node_image(&db, first, DefaultRequest::Make)
            .await
            .unwrap();
        let mut second = ModelKind::default_for(NodeModel::UbuntuLinux).node_config();
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
            NodeModel::CiscoCsr1000,
            NodeModel::CiscoIosl2,
            NodeModel::UbuntuLinux,
        ] {
            let mut image = ModelKind::default_for(model).node_config();
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
            counts.contains(&(NodeModel::CiscoCsr1000, 15)),
            "{counts:?}"
        );
        assert!(counts.contains(&(NodeModel::CiscoIosl2, 15)), "{counts:?}");
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
        for model in [NodeModel::CiscoIse, NodeModel::CiscoAsa] {
            insert_into_older_schema(
                &db,
                &ModelKind::default_for(model).node_config(),
                true,
                "cdrom",
            )
            .await;
        }
        for model in [NodeModel::Generic, NodeModel::CiscoIos] {
            let mut image = ModelKind::default_for(model).node_config();
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
            methods.contains(&(NodeModel::CiscoAsa, Ztp::Cdrom)),
            "{methods:?}"
        );

        let buses: Vec<(NodeModel, DiskBuses)> = list_node_images(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|image| (image.model, image.cdrom.bus))
            .collect();
        assert!(
            buses.contains(&(NodeModel::Generic, DiskBuses::Sata)),
            "{buses:?}"
        );
        assert!(
            buses.contains(&(NodeModel::CiscoIos, DiskBuses::Virtio)),
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
        for model in [NodeModel::JuniperSrxv3, NodeModel::AristaEos] {
            insert_into_older_schema(
                &db,
                &ModelKind::default_for(model).node_config(),
                true,
                "tftp",
            )
            .await;
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
            methods.contains(&(NodeModel::JuniperSrxv3, Ztp::Cdrom)),
            "{methods:?}"
        );
        assert!(
            methods.contains(&(NodeModel::AristaEos, Ztp::Tftp)),
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
            &ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
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
            &ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
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
            let mut image = ModelKind::default_for(model).node_config();
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
        // Three admin protection triggers, the two that keep interfaces used once, the three
        // that keep CPU pins to the pinning rules, and the two that keep device passwords sealed.
        assert_eq!(trigger_count(&db, "%").await, 3 + 2 + 3 + 2);
        assert_eq!(trigger_count(&db, "cpu_pin%").await, 3);
    }

    #[tokio::test]
    async fn apply_schema_folds_every_ztp_pair_into_the_one_setting() {
        // A version 15 database, before the one `ztp` column.
        let db = database_at(14).await;
        for model in models_before_the_ztp_setting() {
            for enable in [false, true] {
                for method in OLD_ZTP_METHODS {
                    let mut image = ModelKind::default_for(model).node_config();
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
            check_ztp(image.model_kind().unwrap(), image.ztp.method).unwrap();
        }
    }

    #[tokio::test]
    async fn apply_schema_keeps_the_settings_this_release_ships_with() {
        // The pairs the model defaults of the release before stored: enabled with the model's
        // method, `none` for every container, and off for unikernels.
        let db = database_at(14).await;
        for model in models_before_the_ztp_setting() {
            let mut image = ModelKind::default_for(model).node_config();
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
            let default = ModelKind::default_for(image.model).node_config().ztp.method;
            assert_eq!(image.ztp.method, default, "{}", image.model);
        }
    }

    #[tokio::test]
    async fn apply_schema_turns_ztp_off_for_a_model_this_build_does_not_know() {
        let db = database_at(14).await;
        let key = insert_into_older_schema(
            &db,
            &ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
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
        for model_kind in ModelKind::all() {
            let shipped = model_kind.node_config();
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
            for (image, default) in [(shipped, true), (edited, false)] {
                let image = NodeConfig { default, ..image };
                stored.push(insert_with_ztp_login(&db, &image, (None, None, false)).await);
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

    /// The number of migrations before the one that stores each node's ready check.
    fn migrations_before_ready_checks() -> usize {
        migrations::ALL
            .iter()
            .position(|batch| *batch == migrations::NODE_READY_CHECK)
            .unwrap()
    }

    #[tokio::test]
    async fn nodes_from_before_ready_checks_get_the_check_the_scanner_ran() {
        // A database from before the ready check was stored, with a node of each kind: what
        // the scanner did with each then is what it does now.
        let db = database_at(migrations_before_ready_checks()).await;
        let mut images = Vec::new();
        for model in [
            NodeModel::UbuntuLinux,
            NodeModel::TalosLinux,
            NodeModel::FrrLinux,
            NodeModel::NanosUnikernel,
        ] {
            images.push((
                model,
                insert_default_before_the_login_drop(&db, model).await,
            ));
        }
        {
            let writer = db.writer().await.unwrap();
            writer
                .conn
                .execute_batch(
                    "INSERT INTO user (id, username, password_hash, is_admin, ssh_keys,
                         created_at, updated_at)
                     VALUES ('u1', 'alice', 'x', 0, '[]', '2026-01-01T00:00:00Z',
                         '2026-01-01T00:00:00Z');
                     INSERT INTO lab (id, lab_id, name, user, loopback_network,
                         management_network, gateway_ipv4, router_ipv4)
                     VALUES ('l1', 'lab-0001', 'Old', 'u1', '127.127.1.0/24', '172.31.1.0/24',
                         '172.31.1.1', '172.31.1.2');",
                )
                .await
                .unwrap();
            for (index, (model, image)) in (0_i64..).zip(&images) {
                writer
                    .conn
                    .execute(
                        "INSERT INTO node (id, name, \"index\", image, lab, state) \
                         VALUES (?1, ?1, ?2, ?3, 'l1', 'running')",
                        (model.to_string(), index, image.as_str()),
                    )
                    .await
                    .unwrap();
            }
        }

        apply_schema(&db).await.unwrap();

        let lab = find_lab(&db, "lab-0001").await.unwrap().unwrap();
        let nodes = list_nodes_by_lab(&db, lab.id.unwrap()).await.unwrap();
        let check_of = |model: NodeModel| {
            nodes
                .iter()
                .find(|node| node.name == model.to_string())
                .unwrap()
                .ready_check
                .clone()
        };
        // Virtual machines keep their model's port; containers and unikernels have none, so
        // running counts as ready. No check is skipped, and none waits for Sherpa's SSH ready
        // marker, which nodes built before Sherpa's Windows sshd config never show.
        assert_eq!(
            check_of(NodeModel::UbuntuLinux),
            ReadyCheck {
                port: Some(SSH_PORT),
                skip: false,
                sshd_marker: false,
            }
        );
        assert_eq!(
            check_of(NodeModel::TalosLinux),
            ReadyCheck {
                port: Some(TALOS_API_PORT),
                skip: false,
                sshd_marker: false,
            }
        );
        assert_eq!(check_of(NodeModel::FrrLinux), ReadyCheck::default());
        assert_eq!(check_of(NodeModel::NanosUnikernel), ReadyCheck::default());
    }

    fn migrations_before_lab_definitions() -> usize {
        migrations::ALL
            .iter()
            .position(|batch| *batch == migrations::LAB_DEFINITION)
            .unwrap()
    }

    #[tokio::test]
    async fn labs_from_before_definitions_have_none_and_keep_their_records() {
        // A database from before lab definitions were stored, with a lab and a node.
        let db = database_at(migrations_before_lab_definitions()).await;
        let image = insert_default_before_the_login_drop(&db, NodeModel::UbuntuLinux).await;
        {
            let writer = db.writer().await.unwrap();
            writer
                .conn
                .execute_batch(
                    "INSERT INTO user (id, username, password_hash, is_admin, ssh_keys,
                         created_at, updated_at)
                     VALUES ('u1', 'alice', 'x', 0, '[]', '2026-01-01T00:00:00Z',
                         '2026-01-01T00:00:00Z');
                     INSERT INTO lab (id, lab_id, name, user, loopback_network,
                         management_network, gateway_ipv4, router_ipv4, description)
                     VALUES ('l1', 'lab-0001', 'Old', 'u1', '127.127.1.0/24', '172.31.1.0/24',
                         '172.31.1.1', '172.31.1.2', 'Kept');",
                )
                .await
                .unwrap();
            writer
                .conn
                .execute(
                    "INSERT INTO node (id, name, \"index\", image, lab, state, memory) \
                     VALUES ('n1', 'dev01', 1, ?1, 'l1', 'running', 2048)",
                    [image.as_str()],
                )
                .await
                .unwrap();
        }

        apply_schema(&db).await.unwrap();
        assert_eq!(version(&db).await, SCHEMA_VERSION);

        let lab = find_lab(&db, "lab-0001").await.unwrap().unwrap();
        assert_eq!(lab.description.as_deref(), Some("Kept"));
        let lab_id = lab.id.clone().unwrap();
        assert_eq!(read_lab_definition(&db, &lab_id).await.unwrap(), None);
        let nodes = list_nodes_by_lab(&db, lab_id).await.unwrap();
        assert_eq!(nodes[0].memory, Some(2048));
    }

    #[tokio::test]
    async fn the_import_mark_is_dropped_and_marked_nodes_are_kept() {
        // A database before the mark was dropped, with a lab whose definition is stored and a
        // node the startup import of 0.6.4 or 0.6.5 marked.
        let before = migrations::ALL
            .iter()
            .position(|batch| *batch == migrations::DROP_NODE_DEFINITION_IMPORTED)
            .unwrap();
        let db = database_at(before).await;
        let image = insert_default_before_the_login_drop(&db, NodeModel::UbuntuLinux).await;
        {
            let writer = db.writer().await.unwrap();
            writer
                .conn
                .execute_batch(
                    "INSERT INTO user (id, username, password_hash, is_admin, ssh_keys,
                         created_at, updated_at)
                     VALUES ('u1', 'alice', 'x', 0, '[]', '2026-01-01T00:00:00Z',
                         '2026-01-01T00:00:00Z');
                     INSERT INTO lab (id, lab_id, name, user, loopback_network,
                         management_network, gateway_ipv4, router_ipv4, has_definition)
                     VALUES ('l1', 'lab-0001', 'Old', 'u1', '127.127.1.0/24', '172.31.1.0/24',
                         '172.31.1.1', '172.31.1.2', 1);",
                )
                .await
                .unwrap();
            writer
                .conn
                .execute(
                    "INSERT INTO node (id, name, \"index\", image, lab, state, \
                         definition_imported) \
                     VALUES ('n1', 'dev01', 1, ?1, 'l1', 'running', 1)",
                    [image.as_str()],
                )
                .await
                .unwrap();
        }

        apply_schema(&db).await.unwrap();
        assert_eq!(version(&db).await, SCHEMA_VERSION);
        let lab = find_lab(&db, "lab-0001").await.unwrap().unwrap();
        let nodes = list_nodes_by_lab(&db, lab.id.clone().unwrap())
            .await
            .unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].name, "dev01");
        let writer = db.writer().await.unwrap();
        let columns = query_count(
            &writer.conn,
            Query::new(
                "SELECT count(*) FROM pragma_table_info('node') \
                 WHERE name = 'definition_imported'",
            )
            .bind([]),
        )
        .await
        .unwrap();
        assert_eq!(columns, 0);
    }

    /// The names of the `node_image` columns `names` that the table has.
    async fn image_columns(db: &Database, names: &[&str]) -> Vec<String> {
        let conn = db.reader().await.unwrap();
        let mut rows = conn
            .query(
                "SELECT name FROM pragma_table_info('node_image') ORDER BY cid",
                (),
            )
            .await
            .unwrap();
        let mut found = Vec::new();
        while let Some(row) = rows.next().await.unwrap() {
            let name = row.get::<String>(0).unwrap();
            if names.contains(&name.as_str()) {
                found.push(name);
            }
        }
        found
    }

    #[tokio::test]
    async fn the_image_ztp_login_columns_and_their_triggers_are_dropped_and_images_kept() {
        // A version 25 database, with an image that has a ZTP username, a sealed password and
        // password auth on, and one with none of them.
        let before = migrations::ALL
            .iter()
            .position(|batch| *batch == migrations::DROP_IMAGE_ZTP_LOGIN)
            .unwrap();
        assert_eq!(before + 1, 25);
        let db = database_at(before).await;
        let sealed = format!("sealed:{}", "a1".repeat(40));
        let with_login = NodeConfig {
            default: true,
            ..ModelKind::default_for(NodeModel::CiscoCat8000).node_config()
        };
        let with_login =
            insert_with_ztp_login(&db, &with_login, (Some("admin"), Some(&sealed), true)).await;
        let without = insert_default_before_the_login_drop(&db, NodeModel::UbuntuLinux).await;
        let login_columns = ["ztp_username", "ztp_password", "ztp_password_auth"];
        assert_eq!(image_columns(&db, &login_columns).await, login_columns);
        assert_eq!(
            trigger_count(&db, "node_image_ztp_password_sealed_%").await,
            2
        );
        // The stored password is the sealed one the triggers allow.
        {
            let conn = db.reader().await.unwrap();
            let stored = query_count(
                &conn,
                Query::new(
                    "SELECT count(*) FROM node_image WHERE ztp_username = 'admin' \
                     AND ztp_password = ?1 AND ztp_password_auth = 1",
                )
                .bind([sealed.as_str().into()]),
            )
            .await
            .unwrap();
            assert_eq!(stored, 1);
        }

        apply_schema(&db).await.unwrap();

        assert_eq!(version(&db).await, SCHEMA_VERSION);
        assert_eq!(
            image_columns(&db, &login_columns).await,
            Vec::<String>::new()
        );
        assert_eq!(
            image_columns(&db, &["ztp"]).await,
            ["ztp"],
            "the ZTP setting stays"
        );
        assert_eq!(
            trigger_count(&db, "node_image_ztp_password_sealed_%").await,
            0
        );
        let images = list_node_images(&db).await.unwrap();
        assert_eq!(images.len(), 2);
        let kept = find_node_image_by_id(&db, with_login.id.clone().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::to_value(&kept).unwrap(),
            serde_json::to_value(&with_login).unwrap()
        );
        assert_eq!(kept.ztp.method, Ztp::Cdrom);
        assert!(kept.default);
        assert!(
            images
                .iter()
                .any(|image| image.id == Some(RecordId::new("node_image", without.clone())))
        );
        // The image can still be edited and stored as today's queries write it.
        let mut edited = kept;
        edited.memory = 8192;
        let updated = update_node_image(&db, edited, DefaultRequest::Keep)
            .await
            .unwrap();
        assert_eq!(updated.memory, 8192);
    }

    #[tokio::test]
    async fn stored_pins_move_to_the_cpu_pin_table_shared_ones_included() {
        // A database from before the `cpu_pin` table: two labs whose nodes already share host
        // CPU 2 on their vCPUs, against the rules, and one with QEMU thread CPUs.
        let before = migrations::ALL
            .iter()
            .position(|batch| *batch == migrations::CPU_PIN)
            .unwrap();
        let db = database_at(before).await;
        let image = insert_default_before_the_login_drop(&db, NodeModel::UbuntuLinux).await;
        {
            let writer = db.writer().await.unwrap();
            writer
                .conn
                .execute_batch(
                    "INSERT INTO user (id, username, password_hash, is_admin, ssh_keys,
                         created_at, updated_at)
                     VALUES ('u1', 'alice', 'x', 0, '[]', '2026-01-01T00:00:00Z',
                         '2026-01-01T00:00:00Z');
                     INSERT INTO lab (id, lab_id, name, user, loopback_network,
                         management_network, gateway_ipv4, router_ipv4)
                     VALUES ('l1', 'lab-0001', 'One', 'u1', '127.127.1.0/24', '172.31.1.0/24',
                         '172.31.1.1', '172.31.1.2'),
                         ('l2', 'lab-0002', 'Two', 'u1', '127.127.2.0/24', '172.31.2.0/24',
                         '172.31.2.1', '172.31.2.2');",
                )
                .await
                .unwrap();
            for (id, index, lab, vcpus, threads) in [
                ("n1", 1, "l1", Some("[3,2]"), Some("[8,24]")),
                ("n2", 1, "l2", Some("[2]"), None),
                ("n3", 2, "l2", None, None),
            ] {
                writer
                    .conn
                    .execute(
                        "INSERT INTO node (id, name, \"index\", image, lab, state, cpu_pinning, \
                         qemu_thread_cpus) VALUES (?1, ?1, ?2, ?3, ?4, 'running', ?5, ?6)",
                        (id, index, image.as_str(), lab, vcpus, threads),
                    )
                    .await
                    .unwrap();
            }
        }

        apply_schema(&db).await.unwrap();
        assert_eq!(version(&db).await, SCHEMA_VERSION);

        let lab = |key: &str| RecordId::new("lab", key);
        let pins = |name: &str, vcpus: &[u16], threads: &[u16]| NodeCpuPins {
            node_name: name.to_string(),
            vcpu_cpus: vcpus.to_vec(),
            qemu_thread_cpus: threads.to_vec(),
        };
        assert_eq!(
            list_cpu_pins(&db).await.unwrap(),
            vec![
                DbCpuPins {
                    lab: lab("l1"),
                    pins: pins("n1", &[2, 3], &[8, 24]),
                },
                DbCpuPins {
                    lab: lab("l2"),
                    pins: pins("n2", &[2], &[]),
                },
            ]
        );
        let writer = db.writer().await.unwrap();
        let columns = query_count(
            &writer.conn,
            Query::new(
                "SELECT count(*) FROM pragma_table_info('node') \
                 WHERE name IN ('cpu_pinning', 'qemu_thread_cpus')",
            )
            .bind([]),
        )
        .await
        .unwrap();
        assert_eq!(columns, 0);
        // The copied overlap stays, and a new one is refused.
        let error = writer
            .conn
            .execute(
                "INSERT INTO cpu_pin (lab, node_name, host_cpu, role) \
                 VALUES ('l2', 'n3', 3, 'vcpu')",
                (),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("already pinned"), "{error}");
    }

    #[tokio::test]
    async fn users_gain_an_unset_device_password_that_must_be_sealed() {
        // A version 26 database with a user, from before device passwords.
        let before = migrations::ALL
            .iter()
            .position(|batch| *batch == migrations::USER_DEVICE_PASSWORD)
            .unwrap();
        assert_eq!(before + 1, 26);
        let db = database_at(before).await;
        assert_eq!(version(&db).await, 26);
        {
            let writer = db.writer().await.unwrap();
            writer
                .conn
                .execute(
                    "INSERT INTO user (id, username, password_hash, is_admin, ssh_keys, \
                     created_at, updated_at) VALUES ('u1', 'alice', 'x', 0, '[]', \
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                    (),
                )
                .await
                .unwrap();
        }

        apply_schema(&db).await.unwrap();

        assert_eq!(version(&db).await, SCHEMA_VERSION);
        assert_eq!(trigger_count(&db, "user_device_password_sealed_%").await, 2);
        let id = RecordId::new("user", "u1");
        assert!(!device_password_set(&db, &id).await.unwrap());
        let user = get_user(&db, "alice").await.unwrap();
        assert_eq!(user.ssh_keys, Vec::<String>::new());
        let writer = db.writer().await.unwrap();
        let error = writer
            .conn
            .execute(
                "UPDATE user SET device_password = 'Granite-Owl42' WHERE id = 'u1'",
                (),
            )
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("user.device_password must be sealed"),
            "{error}"
        );
    }

    /// The position of `migration` in `migrations::ALL`: the number of migrations before it.
    fn migrations_before(migration: &str) -> usize {
        migrations::ALL
            .iter()
            .position(|batch| *batch == migration)
            .unwrap()
    }

    /// A database at `migrations` migrations with a user, a lab and, for each of `logins`, a
    /// node `n<index>` recording that SSH login. The image is stored with today's queries, so
    /// `migrations` is after the image ZTP login columns were dropped.
    async fn database_with_nodes(migrations: usize, logins: &[Option<&str>]) -> Database {
        let db = database_at(migrations).await;
        let image = upsert_node_image(
            &db,
            ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
            DefaultRequest::Keep,
        )
        .await
        .unwrap();
        let image_key = key_of(&image.id.unwrap(), "node_image").unwrap();
        let writer = db.writer().await.unwrap();
        writer
            .conn
            .execute_batch(
                "INSERT INTO user (id, username, password_hash, is_admin, ssh_keys, created_at,
                     updated_at)
                 VALUES ('u1', 'alice', 'x', 0, '[]', '2026-01-01T00:00:00Z',
                     '2026-01-01T00:00:00Z');
                 INSERT INTO lab (id, lab_id, name, user, loopback_network, management_network,
                     gateway_ipv4, router_ipv4)
                 VALUES ('l1', 'lab-0001', 'Old', 'u1', '127.127.1.0/24', '172.31.1.0/24',
                     '172.31.1.1', '172.31.1.2');",
            )
            .await
            .unwrap();
        for (index, login) in (0_i64..).zip(logins) {
            writer
                .conn
                .execute(
                    "INSERT INTO node (id, name, \"index\", image, lab, state, ssh_username) \
                     VALUES (?1, ?1, ?2, ?3, 'l1', 'running', ?4)",
                    (format!("n{index}"), index, image_key.as_str(), *login),
                )
                .await
                .unwrap();
        }
        drop(writer);
        db
    }

    #[tokio::test]
    async fn the_provision_current_user_column_is_dropped_and_labs_kept() {
        // A version 27 database, from before the setting was removed, with a lab that set it.
        let before = migrations_before(migrations::DROP_PROVISION_CURRENT_USER);
        assert_eq!(before + 1, 27);
        let db = database_with_nodes(before, &[Some("alice")]).await;
        {
            let writer = db.writer().await.unwrap();
            writer
                .conn
                .execute(
                    "UPDATE lab SET has_definition = 1, provision_current_user = 1 \
                     WHERE id = 'l1'",
                    (),
                )
                .await
                .unwrap();
        }

        apply_schema(&db).await.unwrap();

        assert!(version(&db).await >= 28);
        let conn = db.reader().await.unwrap();
        let columns = query_count(
            &conn,
            Query::new(
                "SELECT count(*) FROM pragma_table_info('lab') \
                 WHERE name = 'provision_current_user'",
            )
            .bind([]),
        )
        .await
        .unwrap();
        assert_eq!(columns, 0);
        let lab = find_lab(&db, "lab-0001").await.unwrap().unwrap();
        let lab_id = lab.id.unwrap();
        let definition = read_lab_definition(&db, &lab_id).await.unwrap().unwrap();
        assert_eq!(definition.name, "Old");
        assert_eq!(list_nodes_by_lab(&db, lab_id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn nodes_from_before_owner_logins_record_sherpa_and_others_keep_theirs() {
        // A version 28 database: a node with no login recorded, as `up` left nodes that kept
        // only the shared `sherpa` account, and one with the owner's login.
        let before = migrations_before(migrations::LEGACY_NODE_SSH_USERNAME);
        assert_eq!(before + 1, 28);
        let db = database_with_nodes(before, &[None, Some("alice")]).await;
        assert_eq!(version(&db).await, 28);

        apply_schema(&db).await.unwrap();

        assert_eq!(version(&db).await, SCHEMA_VERSION);
        let lab = find_lab(&db, "lab-0001").await.unwrap().unwrap();
        let mut logins: Vec<(String, Option<String>)> = list_nodes_by_lab(&db, lab.id.unwrap())
            .await
            .unwrap()
            .into_iter()
            .map(|node| (node.name, node.ssh_username))
            .collect();
        logins.sort();
        assert_eq!(
            logins,
            [
                ("n0".to_owned(), Some("sherpa".to_owned())),
                ("n1".to_owned(), Some("alice".to_owned())),
            ]
        );
        assert_eq!(count_nodes_by_ssh_username(&db, "sherpa").await.unwrap(), 1);
        // Applying again changes nothing.
        apply_schema(&db).await.unwrap();
        assert_eq!(count_nodes_by_ssh_username(&db, "sherpa").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn sonic_images_at_the_old_default_get_one_data_interface_per_port() {
        // A version 29 database: sonic_linux images at the old default of 52 data interfaces
        // and at a count an administrator set, and another model at 52.
        let before = migrations_before(migrations::FIX_SONIC_DATA_INTERFACE_COUNT);
        assert_eq!(before + 1, 29);
        let db = database_at(before).await;
        for (model, version, count) in [
            (NodeModel::SonicLinux, "old-default", 52),
            (NodeModel::SonicLinux, "admin-set", 40),
            (NodeModel::UbuntuLinux, "many-nics", 52),
        ] {
            let mut image = ModelKind::default_for(model).node_config();
            image.version = version.to_owned();
            image.interfaces.data_count = count;
            upsert_node_image(&db, image, DefaultRequest::Keep)
                .await
                .unwrap();
        }

        apply_schema(&db).await.unwrap();

        assert_eq!(version(&db).await, SCHEMA_VERSION);
        let mut counts: Vec<(NodeModel, String, u8)> = list_node_images(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|image| (image.model, image.version, image.interfaces.data_count))
            .collect();
        counts.sort_by(|a, b| a.1.cmp(&b.1));
        assert_eq!(
            counts,
            [
                (NodeModel::SonicLinux, "admin-set".to_owned(), 40),
                (NodeModel::UbuntuLinux, "many-nics".to_owned(), 52),
                (NodeModel::SonicLinux, "old-default".to_owned(), 32),
            ]
        );
    }

    /// A version 30 database, from before the renames: an image of each removed model name, as
    /// its default, stored under the old name, an image of a model that keeps its name, and a
    /// lab whose stored definition has nodes on five of them. Returns the database and each old
    /// name's image key.
    async fn database_with_old_model_names() -> (Database, Vec<(&'static str, String)>) {
        let before = migrations_before(migrations::RENAME_NODE_MODELS);
        assert_eq!(before + 1, 30);
        let db = database_at(before).await;
        let mut keys = Vec::new();
        for (old, model, kind) in REMOVED_MODELS {
            let image = NodeConfig {
                version: "1.0".to_owned(),
                default: true,
                ..ModelKind::new(model, Some(kind)).unwrap().node_config()
            };
            let image = upsert_node_image(&db, image, DefaultRequest::Keep)
                .await
                .unwrap();
            let key = key_of(&image.id.unwrap(), "node_image").unwrap();
            let writer = db.writer().await.unwrap();
            writer
                .conn
                .execute(
                    "UPDATE node_image SET model = ?1 WHERE id = ?2",
                    (old, key.as_str()),
                )
                .await
                .unwrap();
            keys.push((old, key));
        }
        upsert_node_image(
            &db,
            ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
            DefaultRequest::Keep,
        )
        .await
        .unwrap();
        {
            let writer = db.writer().await.unwrap();
            writer
                .conn
                .execute_batch(
                    "INSERT INTO user (id, username, password_hash, is_admin, ssh_keys,
                         created_at, updated_at)
                     VALUES ('u1', 'alice', 'x', 0, '[]', '2026-01-01T00:00:00Z',
                         '2026-01-01T00:00:00Z');
                     INSERT INTO lab (id, lab_id, name, user, loopback_network,
                         management_network, gateway_ipv4, router_ipv4, has_definition)
                     VALUES ('l1', 'lab-0001', 'Old', 'u1', '127.127.1.0/24', '172.31.1.0/24',
                         '172.31.1.1', '172.31.1.2', 1);",
                )
                .await
                .unwrap();
            for (index, (name, old)) in (0_i64..).zip([
                ("veos1", "arista_veos"),
                ("ceos1", "arista_ceos"),
                ("srx1", "juniper_vsrxv3"),
                ("iosv1", "cisco_iosv"),
                ("vm1", "generic_vm"),
            ]) {
                writer
                    .conn
                    .execute(
                        "INSERT INTO node (id, name, \"index\", image, lab, state) \
                         VALUES (?1, ?1, ?2, ?3, 'l1', 'running')",
                        (name, index, key_of_old(&keys, old)),
                    )
                    .await
                    .unwrap();
            }
        }

        (db, keys)
    }

    /// The key of the image stored under the old model name `old`.
    fn key_of_old(keys: &[(&'static str, String)], old: &str) -> String {
        keys.iter()
            .find(|(name, _)| *name == old)
            .map(|(_, key)| key.clone())
            .unwrap()
    }

    #[tokio::test]
    async fn images_and_lab_definitions_under_the_removed_model_names_take_the_new_names() {
        let (db, keys) = database_with_old_model_names().await;
        let key = |old: &str| key_of_old(&keys, old);

        apply_schema(&db).await.unwrap();

        assert_eq!(version(&db).await, SCHEMA_VERSION);
        let images = list_node_images(&db).await.unwrap();
        assert_eq!(images.len(), REMOVED_MODELS.len() + 1);
        for (old, model, kind) in REMOVED_MODELS {
            let image = find_node_image_by_id(&db, RecordId::new("node_image", key(old)))
                .await
                .unwrap()
                .unwrap();
            assert_eq!((image.model, image.kind), (model, kind), "{old}");
            assert!(image.default, "{old} keeps its default");
        }
        let manifest = read_lab_definition(&db, &RecordId::new("lab", "l1"))
            .await
            .unwrap()
            .unwrap();
        let nodes: Vec<(String, NodeModel, Option<NodeKind>)> = manifest
            .nodes
            .into_iter()
            .map(|node| (node.name, node.model, node.kind))
            .collect();
        assert_eq!(
            nodes,
            [
                ("veos1".to_owned(), NodeModel::AristaEos, None),
                (
                    "ceos1".to_owned(),
                    NodeModel::AristaEos,
                    Some(NodeKind::Container)
                ),
                ("srx1".to_owned(), NodeModel::JuniperSrxv3, None),
                ("iosv1".to_owned(), NodeModel::CiscoIos, None),
                (
                    "vm1".to_owned(),
                    NodeModel::Generic,
                    Some(NodeKind::VirtualMachine)
                ),
            ]
        );
        // Applying again changes nothing.
        apply_schema(&db).await.unwrap();
        assert_eq!(list_node_images(&db).await.unwrap().len(), images.len());
    }
}
