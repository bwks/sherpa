use anyhow::{Context, Result, anyhow};
use shared::data::{NodeConfig, NodeKind, NodeModel, RecordId};
use tracing::instrument;

use crate::Database;
use crate::persistence::{
    NODE_IMAGE_TABLE, enum_text, key_of, node_image_columns, node_image_from_row,
};
use crate::sql::{query_all, query_count, query_optional};

/// List all node_image records from the database ordered by model
#[instrument(skip(db), level = "debug")]
pub async fn list_node_images(db: &Database) -> Result<Vec<NodeConfig>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        concat!(
            "SELECT ",
            node_image_columns!(),
            " FROM node_image ORDER BY model ASC"
        ),
        (),
        node_image_from_row,
    )
    .await
    .context("Failed to query all node_images from database")
}

/// Get node_image by model, kind, and version
#[instrument(skip(db), level = "debug")]
pub async fn get_node_image_by_model_kind_version(
    db: &Database,
    model: &NodeModel,
    kind: &NodeKind,
    version: &str,
) -> Result<Option<NodeConfig>> {
    let conn = db.reader().await?;
    query_optional(
        &conn,
        concat!(
            "SELECT ",
            node_image_columns!(),
            " FROM node_image WHERE model = ?1 AND kind = ?2 AND version = ?3"
        ),
        (
            enum_text(model, "model")?,
            enum_text(kind, "kind")?,
            version,
        ),
        node_image_from_row,
    )
    .await
    .context(format!(
        "Failed to query node_image from database: model={}, kind={}, version={}",
        model, kind, version
    ))
}

/// Get the default node_image for a specific model and kind
#[instrument(skip(db), level = "debug")]
pub async fn get_default_node_image(
    db: &Database,
    model: &NodeModel,
    kind: &NodeKind,
) -> Result<Option<NodeConfig>> {
    let conn = db.reader().await?;
    query_optional(
        &conn,
        concat!(
            "SELECT ",
            node_image_columns!(),
            " FROM node_image WHERE model = ?1 AND kind = ?2 AND \"default\" = 1"
        ),
        (enum_text(model, "model")?, enum_text(kind, "kind")?),
        node_image_from_row,
    )
    .await
    .context(format!(
        "Failed to query default node_image from database: model={}, kind={}",
        model, kind
    ))
}

/// List all node_image records filtered by kind
#[instrument(skip(db), level = "debug")]
pub async fn list_node_images_by_kind(db: &Database, kind: &NodeKind) -> Result<Vec<NodeConfig>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        concat!(
            "SELECT ",
            node_image_columns!(),
            " FROM node_image WHERE kind = ?1 ORDER BY model ASC"
        ),
        [enum_text(kind, "kind")?],
        node_image_from_row,
    )
    .await
    .context(format!(
        "Failed to query node_images by kind from database: kind={}",
        kind
    ))
}

/// Get all versions of a node_image for a specific model and kind
#[instrument(skip(db), level = "debug")]
pub async fn get_node_image_versions(
    db: &Database,
    model: &NodeModel,
    kind: &NodeKind,
) -> Result<Vec<NodeConfig>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        concat!(
            "SELECT ",
            node_image_columns!(),
            " FROM node_image WHERE model = ?1 AND kind = ?2 ORDER BY model ASC"
        ),
        (enum_text(model, "model")?, enum_text(kind, "kind")?),
        node_image_from_row,
    )
    .await
    .context(format!(
        "Failed to query node_image versions from database: model={}, kind={}",
        model, kind
    ))
}

/// Get multiple node_images by a list of RecordIds in a single query
///
/// Results follow the order of `ids`; ids with no record are skipped.
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
        concat!(
            "SELECT ",
            node_image_columns!(),
            " FROM node_image WHERE id IN (SELECT value FROM json_each(?1))"
        ),
        [keys_json],
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

/// Get node_image by RecordId
#[instrument(skip(db), level = "debug")]
pub async fn get_node_image_by_id(db: &Database, id: RecordId) -> Result<Option<NodeConfig>> {
    let conn = db.reader().await?;
    query_optional(
        &conn,
        concat!(
            "SELECT ",
            node_image_columns!(),
            " FROM node_image WHERE id = ?1"
        ),
        [key_of(&id, NODE_IMAGE_TABLE)?],
        node_image_from_row,
    )
    .await
    .context(format!("Failed to query node_image by id: {:?}", id))
}

/// Get node_image from node_model (returns error if not found)
/// This is used internally for image lookups by model.
#[allow(dead_code)]
#[instrument(skip(db), level = "debug")]
pub(crate) async fn get_node_image(db: &Database, node_model: &NodeModel) -> Result<NodeConfig> {
    let conn = db.reader().await?;
    query_optional(
        &conn,
        concat!(
            "SELECT ",
            node_image_columns!(),
            " FROM node_image WHERE model = ?1 LIMIT 1"
        ),
        [enum_text(node_model, "model")?],
        node_image_from_row,
    )
    .await
    .context(format!(
        "Failed to query node_image from database: {node_model}"
    ))?
    .ok_or_else(|| anyhow!("Node image not found for model: {node_model}"))
}

/// Count total number of node_image records in the database
#[instrument(skip(db), level = "debug")]
pub async fn count_node_images(db: &Database) -> Result<usize> {
    let conn = db.reader().await?;
    query_count(&conn, "SELECT count(*) FROM node_image", ())
        .await
        .context("Failed to count node_images from database")
}
