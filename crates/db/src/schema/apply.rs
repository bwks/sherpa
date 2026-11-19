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
        BiosTypes, BootDiskSettings, CdromSettings, ContainerSettings, CpuArchitecture, CpuModels,
        DbCpuPins, DiskBuses, InterfaceDeviceSettings, InterfaceType, KindSettings, LabPhase,
        MachineSettings, MachineType, ModelKind, NodeConfig, NodeCpuPins, NodeKind, NodeModel,
        NodeState, OsVariant, REMOVED_MODELS, ReadyCheck, RecordId, UnikernelBootMode,
        UnikernelCpuSettings, UnikernelMachineSettings, UnikernelSettings, VmCpuSettings,
        VmSettings, Ztp, ZtpSettings,
    };
    use shared::konst::{SSH_PORT, TALOS_API_PORT};
    use shared::validators::check_ztp;
    use turso::Value;

    use super::*;
    use crate::persistence::{enum_text, key_of, new_key};
    use crate::{
        DefaultRequest, LabDetails, LabNetworks, count_nodes_by_ssh_username, create_lab,
        device_password_set, find_lab, find_node_image_by_id, get_user, list_cpu_pins,
        list_node_images, list_nodes_by_lab, open_in_memory, read_lab_definition,
        update_node_image, upsert_node_image,
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

    /// The `node_image` columns from migration 26 until migration 34 gave each kind only its
    /// own, in order, but the last, `ztp`.
    const IMAGE_COLUMNS_BEFORE_ZTP: &str = r#"id, model, version, repo, os_variant, kind, bios, cpu_count, cpu_architecture, cpu_model, machine_type, vmx_enabled, memory, hdd_bus, cdrom, cdrom_bus, data_interface_count, interface_prefix, interface_type, interface_mtu, first_interface_index, management_interface, reserved_interface_count, "default", boot_mode"#;

    /// The values an older release stored in the columns a kind does not use: the defaults new
    /// container images carried, and a 1500 MTU.
    fn unused_vm_columns() -> VmSettings {
        VmSettings {
            os_variant: OsVariant::Unknown,
            cpu: VmCpuSettings::default(),
            machine: MachineSettings::default(),
            boot_disk: BootDiskSettings::default(),
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Virtio,
            },
            interfaces: InterfaceDeviceSettings {
                kind: InterfaceType::Virtio,
                mtu: 1500,
            },
            ztp: ZtpSettings::default(),
        }
    }

    /// An enum stored by its serde name.
    fn text<T: serde::Serialize>(value: &T, field: &str) -> Value {
        Value::from(enum_text(value, field).unwrap())
    }

    /// `image`'s row as the `node_image` table stored it before migration 34, in the order of
    /// `IMAGE_COLUMNS_BEFORE_ZTP` and then `ztp`, with its model named `model`. A VM's columns
    /// are its group's; another kind's own settings are its group's, and the columns it does
    /// not use hold `unused`'s. The interface name columns hold `eth` and 0.
    fn row_before_per_kind(
        image: &NodeConfig,
        key: &str,
        model: &str,
        unused: &VmSettings,
    ) -> Vec<Value> {
        let vm = image.vm().unwrap_or(unused);
        let (architecture, cpu_model, machine_type, device) = match image.unikernel() {
            Some(unikernel) => (
                &unikernel.cpu.architecture,
                &unikernel.cpu.model,
                &unikernel.machine.kind,
                &unikernel.interfaces,
            ),
            None => (
                &vm.cpu.architecture,
                &vm.cpu.model,
                &vm.machine.kind,
                &vm.interfaces,
            ),
        };
        vec![
            Value::from(key),
            Value::from(model),
            Value::from(image.version.as_str()),
            Value::from(
                image
                    .container()
                    .and_then(|container| container.repo.as_deref()),
            ),
            text(&vm.os_variant, "os_variant"),
            text(&image.kind(), "kind"),
            text(&vm.machine.bios, "bios"),
            Value::from(image.cpu.count),
            text(architecture, "cpu_architecture"),
            text(cpu_model, "cpu_model"),
            text(machine_type, "machine_type"),
            Value::from(vm.cpu.vmx),
            Value::from(image.memory),
            text(&vm.boot_disk.bus, "hdd_bus"),
            Value::from(vm.cdrom.image.as_deref()),
            text(&vm.cdrom.bus, "cdrom_bus"),
            Value::from(image.interfaces.data_count),
            Value::from("eth"),
            text(&device.kind, "interface_type"),
            Value::from(device.mtu),
            Value::from(0),
            text(&image.interfaces.management, "management_interface"),
            Value::from(image.interfaces.reserved_count),
            Value::from(image.default),
            image.unikernel().map_or(Value::Null, |unikernel| {
                text(&unikernel.boot_mode, "boot_mode")
            }),
            text(&image.ztp(), "ztp"),
        ]
    }

    /// Insert `image` into a database from migration 26 until migration 34, whose
    /// `node_image` table still holds every kind's columns, with its model named `model`.
    /// `image.default` is stored as it is. Returns the image with its id.
    async fn insert_before_per_kind(db: &Database, image: &NodeConfig, model: &str) -> NodeConfig {
        let key = new_key();
        let values = row_before_per_kind(image, &key, model, &unused_vm_columns());
        let placeholders = (1..=values.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let writer = db.writer().await.unwrap();
        writer
            .conn
            .execute(
                &format!(
                    "INSERT INTO node_image ({IMAGE_COLUMNS_BEFORE_ZTP}, ztp) \
                     VALUES ({placeholders})"
                ),
                values,
            )
            .await
            .unwrap();
        NodeConfig {
            id: Some(RecordId::new("node_image", key)),
            ..image.clone()
        }
    }

    /// The name `image`'s model was stored under before migration 31 renamed the models.
    fn name_before_the_renames(image: &NodeConfig) -> String {
        REMOVED_MODELS
            .iter()
            .find(|(_, model, kind)| *model == image.model && *kind == image.kind())
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
        let mut values = row_before_per_kind(
            image,
            &key,
            &name_before_the_renames(image),
            &unused_vm_columns(),
        );
        // The last column is `ztp`, which the older table does not have.
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
    /// stored password text and the password-auth flag. The columns `image`'s kind does not use
    /// hold `unused`'s. `image.default` is stored as it is.
    /// Returns the image with its id.
    async fn insert_with_ztp_login(
        db: &Database,
        image: &NodeConfig,
        login: (Option<&str>, Option<&str>, bool),
        unused: &VmSettings,
    ) -> NodeConfig {
        let key = new_key();
        let mut values = row_before_per_kind(image, &key, &name_before_the_renames(image), unused);
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
        let stored =
            insert_with_ztp_login(db, &image, (None, None, false), &unused_vm_columns()).await;
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
        if image.kind() != NodeKind::VirtualMachine {
            // The four models with mounted configs are the only ones whose default is not off.
            return image.ztp();
        }
        let stored: Option<Ztp> =
            serde_json::from_value(serde_json::Value::String(method.to_owned())).ok();
        match stored {
            Some(ztp) if ModelKind::default_for(model).ztp_methods().contains(&ztp) => ztp,
            // `volume` on a VM, `none` and `ipxe`: their arms generated nothing.
            Some(Ztp::Volume) | None => Ztp::Disabled,
            Some(_) => image.ztp(),
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
            let KindSettings::VirtualMachine(vm) = &mut image.settings else {
                panic!("{model} is a VM");
            };
            vm.cdrom.bus = DiskBuses::Virtio;
            insert_into_older_schema(&db, &image, true, "cloud-init").await;
        }

        apply_schema(&db).await.unwrap();
        assert_eq!(version(&db).await, SCHEMA_VERSION);

        let methods: Vec<(NodeModel, Ztp)> = list_node_images(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|image| (image.model, image.ztp()))
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
            .map(|image| (image.model, image.vm().unwrap().cdrom.bus.clone()))
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
            .map(|image| (image.model, image.ztp()))
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
                    r#"INSERT INTO node (id, name, "index", image, lab, state) VALUES (?1, ?1, ?2, ?3, 'l1', ?4)"#,
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
                image.ztp(),
                expected,
                "{} stored as {}",
                image.model,
                image.version
            );
            check_ztp(image.model_kind().unwrap(), image.ztp()).unwrap();
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
            let (enable, method) = match image.kind() {
                NodeKind::VirtualMachine => (true, image.ztp().to_string()),
                NodeKind::Container => (true, "none".to_owned()),
                NodeKind::Unikernel => (false, "none".to_owned()),
            };
            insert_into_older_schema(&db, &image, enable, &method).await;
        }

        apply_schema(&db).await.unwrap();

        for image in list_node_images(&db).await.unwrap() {
            let default = ModelKind::default_for(image.model).node_config().ztp();
            assert_eq!(image.ztp(), default, "{}", image.model);
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
    async fn apply_schema_keeps_each_kinds_own_settings_and_drops_the_rest() {
        // A version 17 database. Every model is stored twice: as it ships, and edited to
        // settings other than the defaults, as the web form or an older release could store.
        // The columns a kind does not use hold the edited values too.
        let db = database_at(16).await;
        let shipped_vm = unused_vm_columns();
        let edited_vm = VmSettings {
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
            ..shipped_vm.clone()
        };
        let mut stored = Vec::new();
        for model_kind in ModelKind::all() {
            let shipped = model_kind.node_config();
            let mut edited = NodeConfig {
                version: "edited".to_owned(),
                ..shipped.clone()
            };
            match &mut edited.settings {
                KindSettings::VirtualMachine(vm) => {
                    vm.machine = edited_vm.machine.clone();
                    vm.boot_disk = edited_vm.boot_disk.clone();
                    vm.cdrom = edited_vm.cdrom.clone();
                }
                KindSettings::Unikernel(unikernel) => unikernel.machine.kind = MachineType::Q35,
                KindSettings::Container(_) => {}
            }
            for (image, default) in [(shipped, true), (edited, false)] {
                let image = NodeConfig { default, ..image };
                stored.push(
                    insert_with_ztp_login(&db, &image, (None, None, false), &edited_vm).await,
                );
            }
        }

        apply_schema(&db).await.unwrap();

        let read = list_node_images(&db).await.unwrap();
        assert_eq!(read.len(), stored.len());
        for before in stored {
            let after = read
                .iter()
                .find(|image| image.id == before.id)
                .unwrap_or_else(|| panic!("{} {} is gone", before.model, before.version));
            assert_eq!(
                serde_json::to_value(after).unwrap(),
                serde_json::to_value(&before).unwrap(),
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
                        r#"INSERT INTO node (id, name, "index", image, lab, state) VALUES (?1, ?1, ?2, ?3, 'l1', 'running')"#,
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
                    r#"INSERT INTO node (id, name, "index", image, lab, state, memory) VALUES ('n1', 'dev01', 1, ?1, 'l1', 'running', 2048)"#,
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
                    r#"INSERT INTO node (id, name, "index", image, lab, state, definition_imported) VALUES ('n1', 'dev01', 1, ?1, 'l1', 'running', 1)"#,
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
        let with_login = insert_with_ztp_login(
            &db,
            &with_login,
            (Some("admin"), Some(&sealed), true),
            &unused_vm_columns(),
        )
        .await;
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
        assert_eq!(kept.ztp(), Ztp::Cdrom);
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
                        r#"INSERT INTO node (id, name, "index", image, lab, state, cpu_pinning, qemu_thread_cpus) VALUES (?1, ?1, ?2, ?3, ?4, 'running', ?5, ?6)"#,
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

    #[tokio::test]
    async fn the_loopback_network_is_retired_and_a_new_lab_stores_its_own_id_there() {
        // A version 34 database, from before consoles moved behind sherpad, with a lab that was
        // given a loopback network.
        let before = migrations_before(migrations::RETIRE_LAB_LOOPBACK_NETWORK);
        assert_eq!(before + 1, 34);
        let db = database_at(before).await;
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
        }

        apply_schema(&db).await.unwrap();

        assert_eq!(
            find_lab(&db, "lab-0001").await.unwrap().unwrap().name,
            "Old"
        );
        let networks = LabNetworks {
            management_network: "172.31.2.0/24".to_owned(),
            gateway_ipv4: "172.31.2.1".to_owned(),
            router_ipv4: "172.31.2.2".to_owned(),
            management_network_v6: None,
            loopback_network_v6: None,
            gateway_ipv6: None,
            router_ipv6: None,
        };
        let user = get_user(&db, "alice").await.unwrap();
        create_lab(
            &db,
            "New",
            "lab-0002",
            &user,
            &networks,
            &LabDetails::default(),
        )
        .await
        .unwrap();
        let conn = db.reader().await.unwrap();
        for (lab_id, retired) in [("lab-0001", "127.127.1.0/24"), ("lab-0002", "lab-0002")] {
            let rows = query_count(
                &conn,
                Query::new(
                    "SELECT count(*) FROM lab \
                     WHERE lab_id = ?1 AND retired_loopback_network = ?2",
                )
                .bind([Value::from(lab_id), Value::from(retired)]),
            )
            .await
            .unwrap();
            assert_eq!(rows, 1, "{lab_id}");
        }
    }

    #[tokio::test]
    async fn the_vnc_port_is_retired_and_cleared_and_nodes_still_read_and_write() {
        // A version 35 database, from before VNC moved behind sherpad, with two VMs that were
        // given VNC ports.
        let before = migrations_before(migrations::RETIRE_NODE_VNC_PORT);
        assert_eq!(before + 1, 35);
        let db = database_at(before).await;
        let user = crate::create_user(&db, "alice".to_owned(), "TestPass123!", false, vec![])
            .await
            .unwrap();
        let networks = LabNetworks {
            management_network: "172.31.1.0/24".to_owned(),
            gateway_ipv4: "172.31.1.1".to_owned(),
            router_ipv4: "172.31.1.2".to_owned(),
            management_network_v6: None,
            loopback_network_v6: None,
            gateway_ipv6: None,
            router_ipv6: None,
        };
        let lab = create_lab(
            &db,
            "Old",
            "lab-0001",
            &user,
            &networks,
            &LabDetails::default(),
        )
        .await
        .unwrap();
        let image = upsert_node_image(
            &db,
            ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
            DefaultRequest::Keep,
        )
        .await
        .unwrap();
        for (index, port) in [(1u16, 5900u16), (2, 5901)] {
            let node = crate::create_node(
                &db,
                &format!("node{index}"),
                index,
                image.id.clone().unwrap(),
                lab.id.clone().unwrap(),
            )
            .await
            .unwrap();
            let writer = db.writer().await.unwrap();
            writer
                .conn
                .execute(
                    "UPDATE node SET vnc_port = ?2 WHERE id = ?1",
                    [
                        Value::from(key_of(node.id.as_ref().unwrap(), "node").unwrap()),
                        Value::from(i64::from(port)),
                    ],
                )
                .await
                .unwrap();
        }

        apply_schema(&db).await.unwrap();

        let conn = db.reader().await.unwrap();
        let cleared = query_count(
            &conn,
            Query::new("SELECT count(*) FROM node WHERE retired_vnc_port IS NULL").bind([]),
        )
        .await
        .unwrap();
        assert_eq!(cleared, 2);
        drop(conn);
        let nodes = list_nodes_by_lab(&db, lab.id.clone().unwrap())
            .await
            .unwrap();
        assert_eq!(nodes.len(), 2);
        // A node's resources are written without the column.
        let resources = shared::data::NodeResources {
            cpu_count: Some(2),
            memory: Some(1024),
        };
        let updated = crate::update_node_resources(&db, nodes[0].id.clone().unwrap(), &resources)
            .await
            .unwrap();
        assert_eq!(updated.memory, Some(1024));
    }

    /// The position of `migration` in `migrations::ALL`: the number of migrations before it.
    fn migrations_before(migration: &str) -> usize {
        migrations::ALL
            .iter()
            .position(|batch| *batch == migration)
            .unwrap()
    }

    /// A database at `migrations` migrations with a user, a lab and, for each of `logins`, a
    /// node `n<index>` recording that SSH login. `migrations` is after the image ZTP login
    /// columns were dropped and before each kind got its own image columns.
    async fn database_with_nodes(migrations: usize, logins: &[Option<&str>]) -> Database {
        let db = database_at(migrations).await;
        let image = insert_before_per_kind(
            &db,
            &ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
            "ubuntu_linux",
        )
        .await;
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
                    r#"INSERT INTO node (id, name, "index", image, lab, state, ssh_username) VALUES (?1, ?1, ?2, ?3, 'l1', 'running', ?4)"#,
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
        for (model, version, count, default) in [
            (NodeModel::SonicLinux, "old-default", 52, true),
            (NodeModel::SonicLinux, "admin-set", 40, false),
            (NodeModel::UbuntuLinux, "many-nics", 52, true),
        ] {
            let mut image = ModelKind::default_for(model).node_config();
            image.version = version.to_owned();
            image.interfaces.data_count = count;
            image.default = default;
            insert_before_per_kind(&db, &image, &model.to_string()).await;
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

    #[tokio::test]
    async fn srx_images_on_tftp_move_to_cdrom_and_other_settings_are_kept() {
        // A version 32 database: vSRX images on each setting it could hold, and a Junos
        // Evolved image on TFTP, which that model still takes.
        let before = migrations_before(migrations::SRX_ZTP_CDROM_ONLY);
        assert_eq!(before + 1, 32);
        let db = database_at(before).await;
        for (model, version, ztp) in [
            (NodeModel::JuniperSrxv3, "on-tftp", "tftp"),
            (NodeModel::JuniperSrxv3, "on-cdrom", "cdrom"),
            (NodeModel::JuniperSrxv3, "off", "disabled"),
            (NodeModel::JuniperEvolved, "evolved", "tftp"),
        ] {
            let mut image = ModelKind::default_for(model).node_config();
            image.version = version.to_owned();
            image.default = matches!(version, "on-tftp" | "evolved");
            // Stored as the release before this one could: today's writes refuse vSRX on TFTP.
            let image = insert_before_per_kind(&db, &image, &model.to_string()).await;
            let key = key_of(&image.id.unwrap(), "node_image").unwrap();
            let writer = db.writer().await.unwrap();
            writer
                .conn
                .execute(
                    "UPDATE node_image SET ztp = ?1 WHERE id = ?2",
                    (ztp, key.as_str()),
                )
                .await
                .unwrap();
        }

        apply_schema(&db).await.unwrap();

        assert_eq!(version(&db).await, SCHEMA_VERSION);
        // vSRX is a VM, so its method is in its VM group.
        let mut methods: Vec<(NodeModel, String, Ztp)> = list_node_images(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|image| {
                let method = image.vm().unwrap().ztp.method;
                (image.model, image.version, method)
            })
            .collect();
        methods.sort_by(|a, b| a.1.cmp(&b.1));
        assert_eq!(
            methods,
            [
                (NodeModel::JuniperEvolved, "evolved".to_owned(), Ztp::Tftp),
                (NodeModel::JuniperSrxv3, "off".to_owned(), Ztp::Disabled),
                (NodeModel::JuniperSrxv3, "on-cdrom".to_owned(), Ztp::Cdrom),
                (NodeModel::JuniperSrxv3, "on-tftp".to_owned(), Ztp::Cdrom),
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
            let image = insert_before_per_kind(&db, &image, old).await;
            keys.push((old, key_of(&image.id.unwrap(), "node_image").unwrap()));
        }
        insert_before_per_kind(
            &db,
            &ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
            "ubuntu_linux",
        )
        .await;
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
                        r#"INSERT INTO node (id, name, "index", image, lab, state) VALUES (?1, ?1, ?2, ?3, 'l1', 'running')"#,
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
            assert_eq!((image.model, image.kind()), (model, kind), "{old}");
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

    /// Insert a `node_image` row into a database from before migration 34: the columns `values`
    /// names hold its values, and the rest those of a container row an older release stored.
    async fn insert_old_row(db: &Database, values: &[(&str, Value)]) {
        let mut row: Vec<(&str, Value)> = vec![
            ("id", Value::from(new_key())),
            ("repo", Value::Null),
            ("os_variant", text(&OsVariant::Unknown, "os_variant")),
            ("bios", text(&BiosTypes::SeaBios, "bios")),
            ("cpu_count", Value::from(1)),
            ("cpu_architecture", Value::from("x86_64")),
            ("cpu_model", Value::from("host-model")),
            ("machine_type", text(&MachineType::Pc, "machine_type")),
            ("vmx_enabled", Value::from(0)),
            ("memory", Value::from(1024)),
            ("hdd_bus", text(&DiskBuses::Virtio, "hdd_bus")),
            ("cdrom", Value::Null),
            ("cdrom_bus", text(&DiskBuses::Virtio, "cdrom_bus")),
            ("data_interface_count", Value::from(1)),
            ("interface_prefix", Value::from("eth")),
            (
                "interface_type",
                text(&InterfaceType::MacVlan, "interface_type"),
            ),
            ("interface_mtu", Value::from(1500)),
            ("first_interface_index", Value::from(0)),
            ("management_interface", Value::from("eth0")),
            ("reserved_interface_count", Value::from(0)),
            ("default", Value::from(1)),
            ("boot_mode", Value::Null),
            ("ztp", Value::from("disabled")),
        ];
        for (column, value) in values {
            match row.iter_mut().find(|(name, _)| name == column) {
                Some(cell) => cell.1 = value.clone(),
                None => row.push((column, value.clone())),
            }
        }
        let columns = row
            .iter()
            .map(|(name, _)| format!(r#""{name}""#))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = (1..=row.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let writer = db.writer().await.unwrap();
        writer
            .conn
            .execute(
                &format!("INSERT INTO node_image ({columns}) VALUES ({placeholders})"),
                row.into_iter().map(|(_, value)| value).collect::<Vec<_>>(),
            )
            .await
            .unwrap();
    }

    /// A database from before migration 34 with a VM edited away from its model's defaults, a
    /// container whose unused columns hold values no container read, and two unikernels, one
    /// stored without its boot mode.
    async fn database_with_one_image_of_each_kind() -> Database {
        let before = migrations_before(migrations::PER_KIND_IMAGE_SETTINGS);
        let db = database_at(before).await;
        // A VM edited away from its model's defaults.
        insert_old_row(
            &db,
            &[
                ("model", Value::from("cisco_cat9000")),
                ("version", Value::from("17.12")),
                ("kind", Value::from("virtual_machine")),
                ("os_variant", text(&OsVariant::Iosxe, "os_variant")),
                ("bios", text(&BiosTypes::Uefi, "bios")),
                ("cpu_count", Value::from(4)),
                ("cpu_model", Value::from("host-passthrough")),
                ("machine_type", text(&MachineType::Q35, "machine_type")),
                ("vmx_enabled", Value::from(1)),
                ("memory", Value::from(18432)),
                ("hdd_bus", text(&DiskBuses::Sata, "hdd_bus")),
                ("cdrom", Value::from("extra.iso")),
                ("cdrom_bus", text(&DiskBuses::Ide, "cdrom_bus")),
                ("data_interface_count", Value::from(8)),
                ("interface_prefix", Value::from("Gig0/0/")),
                (
                    "interface_type",
                    text(&InterfaceType::E1000, "interface_type"),
                ),
                ("interface_mtu", Value::from(9000)),
                ("first_interface_index", Value::from(1)),
                ("management_interface", Value::from("GigabitEthernet0/0")),
                ("ztp", Value::from("cdrom")),
            ],
        )
        .await;
        // A container whose unused columns hold values no container read.
        insert_old_row(
            &db,
            &[
                ("model", Value::from("frr_linux")),
                ("version", Value::from("10.2")),
                ("kind", Value::from("container")),
                ("repo", Value::from("quay.io/frrouting/frr")),
                ("os_variant", text(&OsVariant::Linux, "os_variant")),
                ("vmx_enabled", Value::from(1)),
                (
                    "interface_type",
                    text(&InterfaceType::Virtio, "interface_type"),
                ),
                ("interface_mtu", Value::from(9000)),
                ("memory", Value::from(512)),
                ("data_interface_count", Value::from(8)),
                ("ztp", Value::from("volume")),
            ],
        )
        .await;
        // Two unikernels: one with its boot mode, one stored without.
        for (version, boot_mode) in [("1.0", Value::from("disk_boot")), ("0.9", Value::Null)] {
            insert_old_row(
                &db,
                &[
                    ("model", Value::from("nanos_unikernel")),
                    ("version", Value::from(version)),
                    ("kind", Value::from("unikernel")),
                    ("default", Value::from(i64::from(version == "1.0"))),
                    ("machine_type", text(&MachineType::Q35, "machine_type")),
                    (
                        "interface_type",
                        text(&InterfaceType::Virtio, "interface_type"),
                    ),
                    ("bios", text(&BiosTypes::Uefi, "bios")),
                    ("memory", Value::from(512)),
                    ("data_interface_count", Value::from(0)),
                    ("boot_mode", boot_mode),
                ],
            )
            .await;
        }
        db
    }

    #[tokio::test]
    async fn each_image_keeps_only_its_kinds_settings_after_the_per_kind_migration() {
        let db = database_with_one_image_of_each_kind().await;
        apply_schema(&db).await.unwrap();

        let images = list_node_images(&db).await.unwrap();
        assert_eq!(images.len(), 4);
        let image = |model: NodeModel, version: &str| {
            images
                .iter()
                .find(|image| image.model == model && image.version == version)
                .unwrap()
        };
        let vm = image(NodeModel::CiscoCat9000, "17.12");
        assert_eq!(vm.cpu.count, 4);
        assert_eq!(vm.memory, 18432);
        assert_eq!(vm.interfaces.data_count, 8);
        assert_eq!(
            vm.settings,
            KindSettings::VirtualMachine(VmSettings {
                os_variant: OsVariant::Iosxe,
                cpu: VmCpuSettings {
                    architecture: CpuArchitecture::X86_64,
                    model: CpuModels::HostPassthrough,
                    vmx: true,
                },
                machine: MachineSettings {
                    kind: MachineType::Q35,
                    bios: BiosTypes::Uefi,
                },
                boot_disk: BootDiskSettings {
                    bus: DiskBuses::Sata,
                },
                cdrom: CdromSettings {
                    image: Some("extra.iso".to_owned()),
                    bus: DiskBuses::Ide,
                },
                interfaces: InterfaceDeviceSettings {
                    kind: InterfaceType::E1000,
                    mtu: 9000,
                },
                ztp: ZtpSettings { method: Ztp::Cdrom },
            })
        );
        let container = image(NodeModel::FrrLinux, "10.2");
        assert_eq!(container.memory, 512);
        assert_eq!(
            container.settings,
            KindSettings::Container(ContainerSettings {
                repo: Some("quay.io/frrouting/frr".to_owned()),
                ztp: ZtpSettings {
                    method: Ztp::Volume
                },
            })
        );
        for (version, boot_mode) in [
            ("1.0", UnikernelBootMode::DiskBoot),
            ("0.9", UnikernelBootMode::DirectKernel),
        ] {
            assert_eq!(
                image(NodeModel::NanosUnikernel, version).settings,
                KindSettings::Unikernel(UnikernelSettings {
                    boot_mode,
                    cpu: UnikernelCpuSettings {
                        architecture: CpuArchitecture::X86_64,
                        model: CpuModels::HostModel,
                    },
                    machine: UnikernelMachineSettings {
                        kind: MachineType::Q35,
                    },
                    interfaces: InterfaceDeviceSettings {
                        kind: InterfaceType::Virtio,
                        mtu: 1500,
                    },
                }),
                "{version}"
            );
        }

        // Applying again changes nothing.
        apply_schema(&db).await.unwrap();
        assert_eq!(list_node_images(&db).await.unwrap(), images);
    }

    #[tokio::test]
    async fn the_per_kind_columns_are_set_only_for_their_kinds_and_the_name_columns_are_gone() {
        let db = database_with_one_image_of_each_kind().await;
        apply_schema(&db).await.unwrap();

        let writer = db.writer().await.unwrap();
        for column in ["interface_prefix", "first_interface_index"] {
            let count = query_count(
                &writer.conn,
                Query::new("SELECT count(*) FROM pragma_table_info('node_image') WHERE name = ?1")
                    .bind([column.into()]),
            )
            .await
            .unwrap();
            assert_eq!(count, 0, "{column}");
        }
        for refused in [
            "UPDATE node_image SET bios = 'uefi' WHERE kind = 'container'",
            "UPDATE node_image SET bios = NULL WHERE kind = 'virtual_machine'",
            "UPDATE node_image SET interface_mtu = 1500 WHERE kind = 'container'",
            "UPDATE node_image SET interface_mtu = 100 WHERE kind = 'virtual_machine'",
            "UPDATE node_image SET ztp = 'disabled' WHERE kind = 'unikernel'",
            "UPDATE node_image SET ztp = NULL WHERE kind = 'container'",
            "UPDATE node_image SET repo = 'x' WHERE kind = 'virtual_machine'",
            "UPDATE node_image SET boot_mode = NULL WHERE kind = 'unikernel'",
            "UPDATE node_image SET os_variant = 'linux' WHERE kind = 'unikernel'",
        ] {
            assert!(writer.conn.execute(refused, ()).await.is_err(), "{refused}");
        }
    }
}
