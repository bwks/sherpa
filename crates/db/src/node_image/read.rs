use anyhow::{Context, Result};
use shared::data::{NodeConfig, NodeKind, NodeModel, RecordId};
use tracing::instrument;

use crate::Database;
use crate::persistence::{NODE_IMAGE_TABLE, enum_text, key_of, node_image_from_row};
use crate::queries;
use crate::sql::{query_all, query_count, query_optional};

/// List all `node_image` records from the database ordered by model
///
/// # Errors
///
/// Fails when the database cannot be read.
#[instrument(skip(db), level = "debug")]
pub async fn list_node_images(db: &Database) -> Result<Vec<NodeConfig>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        queries::node_image::LIST.bind([]),
        node_image_from_row,
    )
    .await
    .context("Failed to query all node_images from database")
}

/// Get `node_image` by model, kind, and version
///
/// # Errors
///
/// Fails when the database cannot be read; a missing image is `Ok(None)`.
#[instrument(skip(db), level = "debug")]
pub async fn find_node_image_by_model_kind_version(
    db: &Database,
    model: &NodeModel,
    kind: &NodeKind,
    version: &str,
) -> Result<Option<NodeConfig>> {
    let conn = db.reader().await?;
    query_optional(
        &conn,
        queries::node_image::FIND_BY_MODEL_KIND_VERSION.bind([
            enum_text(model, "model")?.into(),
            enum_text(kind, "kind")?.into(),
            version.into(),
        ]),
        node_image_from_row,
    )
    .await
    .context(format!(
        "Failed to query node_image from database: model={model}, kind={kind}, version={version}"
    ))
}

/// Get the default `node_image` for a specific model and kind
///
/// # Errors
///
/// Fails when the database cannot be read; a model with no default image is `Ok(None)`.
#[instrument(skip(db), level = "debug")]
pub async fn find_default_node_image(
    db: &Database,
    model: &NodeModel,
    kind: &NodeKind,
) -> Result<Option<NodeConfig>> {
    let conn = db.reader().await?;
    query_optional(
        &conn,
        queries::node_image::FIND_DEFAULT.bind([
            enum_text(model, "model")?.into(),
            enum_text(kind, "kind")?.into(),
        ]),
        node_image_from_row,
    )
    .await
    .context(format!(
        "Failed to query default node_image from database: model={model}, kind={kind}"
    ))
}

/// List all `node_image` records filtered by kind
///
/// # Errors
///
/// Fails when the database cannot be read.
#[instrument(skip(db), level = "debug")]
pub async fn list_node_images_by_kind(db: &Database, kind: &NodeKind) -> Result<Vec<NodeConfig>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        queries::node_image::LIST_BY_KIND.bind([enum_text(kind, "kind")?.into()]),
        node_image_from_row,
    )
    .await
    .context(format!(
        "Failed to query node_images by kind from database: kind={kind}"
    ))
}

/// Get all versions of a `node_image` for a specific model and kind
///
/// # Errors
///
/// Fails when the database cannot be read.
#[instrument(skip(db), level = "debug")]
pub async fn get_node_image_versions(
    db: &Database,
    model: &NodeModel,
    kind: &NodeKind,
) -> Result<Vec<NodeConfig>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        queries::node_image::LIST_BY_MODEL_AND_KIND.bind([
            enum_text(model, "model")?.into(),
            enum_text(kind, "kind")?.into(),
        ]),
        node_image_from_row,
    )
    .await
    .context(format!(
        "Failed to query node_image versions from database: model={model}, kind={kind}"
    ))
}

/// Get multiple `node_images` by a list of `RecordIds` in a single query
///
/// Results follow the order of `ids`; ids with no record are skipped.
///
/// # Errors
///
/// Fails when the database cannot be read.
#[instrument(skip(db), level = "debug")]
pub async fn list_node_images_by_ids(db: &Database, ids: Vec<RecordId>) -> Result<Vec<NodeConfig>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut keys = Vec::with_capacity(ids.len());
    for id in &ids {
        keys.push(key_of(id, NODE_IMAGE_TABLE)?);
    }
    let keys_json = serde_json::to_string(&keys).context("Failed to encode node_image ids")?;

    let conn = db.reader().await?;
    let found = query_all(
        &conn,
        queries::node_image::LIST_BY_IDS.bind([keys_json.into()]),
        node_image_from_row,
    )
    .await
    .context("Failed to batch query node_images by ids")?;

    let mut ordered = Vec::with_capacity(ids.len());
    for id in &ids {
        if let Some(config) = found.iter().find(|config| config.id.as_ref() == Some(id)) {
            ordered.push(config.clone());
        }
    }
    Ok(ordered)
}

/// Get `node_image` by `RecordId`
///
/// # Errors
///
/// Fails when the database cannot be read; a missing image is `Ok(None)`.
#[instrument(skip(db), level = "debug")]
pub async fn find_node_image_by_id(db: &Database, id: RecordId) -> Result<Option<NodeConfig>> {
    let conn = db.reader().await?;
    query_optional(
        &conn,
        queries::node_image::FIND_BY_ID.bind([key_of(&id, NODE_IMAGE_TABLE)?.into()]),
        node_image_from_row,
    )
    .await
    .context(format!("Failed to query node_image by id: {id:?}"))
}

/// Count total number of `node_image` records in the database
///
/// # Errors
///
/// Fails when the database cannot be read.
#[instrument(skip(db), level = "debug")]
pub async fn count_node_images(db: &Database) -> Result<usize> {
    let conn = db.reader().await?;
    query_count(&conn, queries::node_image::COUNT.bind([]))
        .await
        .context("Failed to count node_images from database")
}
