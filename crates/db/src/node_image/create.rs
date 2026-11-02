use anyhow::{Context, Result, anyhow};
use shared::data::NodeConfig;
use tracing::instrument;
use turso::Connection;

use crate::Database;
use crate::persistence::{
    NODE_IMAGE_TABLE, enum_text, key_of, new_key, node_image_columns, node_image_from_row,
    node_image_values,
};
use crate::sql::query_optional;

/// Create a node_image record in the database with auto-generated ID
#[instrument(skip(db), level = "debug")]
pub async fn create_node_image(db: &Database, config: NodeConfig) -> Result<NodeConfig> {
    let writer = db.writer().await?;
    insert_node_image(&writer.conn, &config).await
}

/// Upsert a node_image record (create if not exists, update if exists)
///
/// The record is matched on its unique `(model, kind, version)`. If config.default is true,
/// it also unsets default on the other versions of the same (model, kind) combination.
/// It runs in one transaction under the write lock, so it is atomic.
/// New records get a generated ID; an existing record keeps its ID.
#[instrument(skip(db), level = "debug")]
pub async fn upsert_node_image(db: &Database, config: NodeConfig) -> Result<NodeConfig> {
    let mut writer = db.writer().await?;
    let tx = writer
        .conn
        .transaction()
        .await
        .context("Failed to start the node_image upsert transaction")?;

    let model = enum_text(&config.model, "model")?;
    let kind = enum_text(&config.kind, "kind")?;

    // If setting default=true, first unset default on other versions of same (model, kind)
    if config.default {
        tx.execute(
            "UPDATE node_image SET \"default\" = 0 WHERE model = ?1 AND kind = ?2 AND version != ?3",
            (model.as_str(), kind.as_str(), config.version.as_str()),
        )
        .await
        .context(format!(
            "Failed to unset default flag on other versions: model={}, kind={}",
            config.model, config.kind
        ))?;
    }

    // Try to find existing config by (model, kind, version) using the unique constraint
    let existing = query_optional(
        &tx,
        concat!(
            "SELECT ",
            node_image_columns!(),
            " FROM node_image WHERE model = ?1 AND kind = ?2 AND version = ?3"
        ),
        (model.as_str(), kind.as_str(), config.version.as_str()),
        node_image_from_row,
    )
    .await
    .context(format!(
        "Failed to query existing node_image: model={}, kind={}, version={}",
        config.model, config.kind, config.version
    ))?;

    let result = match existing {
        Some(existing_config) => {
            // Update existing record, preserving its ID
            let record_id = existing_config
                .id
                .context("Existing node_image record is missing its ID")?;
            let key = key_of(&record_id, NODE_IMAGE_TABLE)?;
            let updated = replace_node_image(&tx, &config, &key).await?;
            updated.ok_or_else(|| {
                anyhow!(
                    "Database returned no record after updating node_image: model={}, kind={}, version={}",
                    config.model,
                    config.kind,
                    config.version
                )
            })?
        }
        None => insert_node_image(&tx, &config).await?,
    };

    tx.commit()
        .await
        .context("Failed to commit the node_image upsert transaction")?;
    Ok(result)
}

/// Insert `config` as a new record with a generated ID.
async fn insert_node_image(conn: &Connection, config: &NodeConfig) -> Result<NodeConfig> {
    let created = query_optional(
        conn,
        concat!(
            "INSERT INTO node_image (",
            node_image_columns!(),
            ") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
             ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31) RETURNING ",
            node_image_columns!()
        ),
        node_image_values(config, &new_key())?,
        node_image_from_row,
    )
    .await
    .context(format!(
        "Failed to create node_image: model={}, kind={}, version={}",
        config.model, config.kind, config.version
    ))?;

    created.ok_or_else(|| {
        anyhow!(
            "Database returned no record after creating node_image: model={}, kind={}, version={}",
            config.model,
            config.kind,
            config.version
        )
    })
}

/// Replace every field of the record `key` with those of `config`. `None` if it does not exist.
pub(crate) async fn replace_node_image(
    conn: &Connection,
    config: &NodeConfig,
    key: &str,
) -> Result<Option<NodeConfig>> {
    query_optional(
        conn,
        concat!(
            "UPDATE node_image SET model = ?2, version = ?3, repo = ?4, os_variant = ?5, \
             kind = ?6, bios = ?7, cpu_count = ?8, cpu_architecture = ?9, cpu_model = ?10, \
             machine_type = ?11, vmx_enabled = ?12, memory = ?13, hdd_bus = ?14, cdrom = ?15, \
             cdrom_bus = ?16, ztp_enable = ?17, ztp_method = ?18, ztp_username = ?19, \
             ztp_password = ?20, ztp_password_auth = ?21, data_interface_count = ?22, \
             interface_prefix = ?23, interface_type = ?24, interface_mtu = ?25, \
             first_interface_index = ?26, dedicated_management_interface = ?27, \
             management_interface = ?28, reserved_interface_count = ?29, \"default\" = ?30, \
             boot_mode = ?31 WHERE id = ?1 RETURNING ",
            node_image_columns!()
        ),
        node_image_values(config, key)?,
        node_image_from_row,
    )
    .await
    .context(format!(
        "Failed to update node_image: model={}, kind={}, version={}",
        config.model, config.kind, config.version
    ))
}
