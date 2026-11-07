use anyhow::{Context, Result, anyhow};
use shared::data::NodeConfig;
use tracing::instrument;
use turso::Connection;

use crate::Database;
use crate::persistence::{
    NODE_IMAGE_TABLE, enum_text, key_of, new_key, node_image_from_row, node_image_values,
};
use crate::queries;
use crate::sql::query_optional;

/// Upsert a `node_image` record (create if not exists, update if exists)
///
/// The record is matched on its unique `(model, kind, version)`. If config.default is true,
/// it also unsets default on the other versions of the same (model, kind) combination.
/// It runs in one transaction under the write lock, so it is atomic.
/// New records get a generated ID; an existing record keeps its ID.
///
/// # Errors
///
/// Fails when the image cannot be written, as when one of its values is not one this build allows.
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
            queries::node_image::CLEAR_DEFAULT_EXCEPT_VERSION,
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
        queries::node_image::FIND_BY_MODEL_KIND_VERSION,
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
        queries::node_image::CREATE,
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
        queries::node_image::UPDATE,
        node_image_values(config, key)?,
        node_image_from_row,
    )
    .await
    .context(format!(
        "Failed to update node_image: model={}, kind={}, version={}",
        config.model, config.kind, config.version
    ))
}
