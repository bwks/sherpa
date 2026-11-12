use anyhow::{Context, Result, anyhow};
use shared::data::NodeConfig;
use shared::validators::check_ztp;
use tracing::instrument;
use turso::Connection;

use crate::Database;
use crate::node_image::default::{DefaultRequest, claim_default};
use crate::persistence::{
    NODE_IMAGE_TABLE, enum_text, key_of, new_key, node_image_from_row, node_image_values,
};
use crate::queries;
use crate::sql::query_optional;

/// Record `config` as an image, or keep the record its `(model, kind, version)` already has.
///
/// A new version is inserted with `config`'s settings. An existing version keeps its settings and
/// its ID, so an import of a version again does not undo an admin's edits. Either way the
/// `default` flag is decided by `request` (see [`DefaultRequest`]); `config.default` is ignored.
/// It runs in one transaction under the write lock, so the model is never left with no default
/// or two.
///
/// # Errors
///
/// Fails when the image's ZTP setting is not one its model takes, when the image cannot be
/// written, as when one of its values is not one this build allows,
/// and with [`LastDefaultImage`](crate::LastDefaultImage) when `request` is
/// [`DefaultRequest::Unset`] on the model's only default.
#[instrument(skip(db), level = "debug")]
pub async fn upsert_node_image(
    db: &Database,
    mut config: NodeConfig,
    request: DefaultRequest,
) -> Result<NodeConfig> {
    check_ztp(config.model, config.ztp.mode).context(format!(
        "Refusing to store node_image: model={}, kind={}, version={}",
        config.model, config.kind, config.version
    ))?;
    let mut writer = db.writer().await?;
    let tx = writer
        .conn
        .transaction()
        .await
        .context("Failed to start the node_image upsert transaction")?;

    let model = enum_text(&config.model, "model")?;
    let kind = enum_text(&config.kind, "kind")?;
    let existing = query_optional(
        &tx,
        queries::node_image::FIND_BY_MODEL_KIND_VERSION.bind([
            model.as_str().into(),
            kind.as_str().into(),
            config.version.as_str().into(),
        ]),
        node_image_from_row,
    )
    .await
    .context(format!(
        "Failed to query existing node_image: model={}, kind={}, version={}",
        config.model, config.kind, config.version
    ))?;
    let is_default =
        claim_default(&tx, config.model, &config.kind, &config.version, request).await?;

    let result = match existing {
        Some(existing) if existing.default == is_default => existing,
        Some(mut existing) => {
            existing.default = is_default;
            let record_id = existing
                .id
                .clone()
                .context("Existing node_image record is missing its ID")?;
            let key = key_of(&record_id, NODE_IMAGE_TABLE)?;
            replace_node_image(&tx, &existing, &key)
                .await?
                .ok_or_else(|| {
                    anyhow!(
                        "Database returned no record after updating node_image: model={}, kind={}, version={}",
                        existing.model,
                        existing.kind,
                        existing.version
                    )
                })?
        }
        None => {
            config.default = is_default;
            config.id = None;
            insert_node_image(&tx, &config).await?
        }
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
        queries::node_image::CREATE.bind(node_image_values(config, &new_key())?),
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
        queries::node_image::UPDATE.bind(node_image_values(config, key)?),
        node_image_from_row,
    )
    .await
    .context(format!(
        "Failed to update node_image: model={}, kind={}, version={}",
        config.model, config.kind, config.version
    ))
}
