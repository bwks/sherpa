use anyhow::{Context, Result, anyhow};
use shared::data::{NodeConfig, NodeKind, NodeModel, RecordId};
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;
use tracing::instrument;

use crate::persistence::{NodeImageRow, to_surreal_id};

/// List all node_image records from the database ordered by model
#[instrument(skip(db), level = "debug")]
pub async fn list_node_images(db: &Arc<Surreal<Client>>) -> Result<Vec<NodeConfig>> {
    let mut response = db
        .query("SELECT * FROM node_image ORDER BY model ASC")
        .await
        .context("Failed to query all node_images from database")?;

    let configs: Vec<NodeImageRow> = response.take(0)?;
    configs.into_iter().map(NodeConfig::try_from).collect()
}

/// Get node_image by model, kind, and version
#[instrument(skip(db), level = "debug")]
pub async fn get_node_image_by_model_kind_version(
    db: &Arc<Surreal<Client>>,
    model: &NodeModel,
    kind: &NodeKind,
    version: &str,
) -> Result<Option<NodeConfig>> {
    let mut response = db
        .query("SELECT * FROM ONLY node_image WHERE model = $model AND kind = $kind AND version = $version")
        .bind(("model", model.to_string()))
        .bind(("kind", kind.to_string()))
        .bind(("version", version.to_string()))
        .await
        .context(format!(
            "Failed to query node_image from database: model={}, kind={}, version={}",
            model, kind, version
        ))?;

    let config: Option<NodeImageRow> = response.take(0)?;
    config.map(NodeConfig::try_from).transpose()
}

/// Get the default node_image for a specific model and kind
#[instrument(skip(db), level = "debug")]
pub async fn get_default_node_image(
    db: &Arc<Surreal<Client>>,
    model: &NodeModel,
    kind: &NodeKind,
) -> Result<Option<NodeConfig>> {
    let mut response = db
        .query("SELECT * FROM ONLY node_image WHERE model = $model AND kind = $kind AND default = true")
        .bind(("model", model.to_string()))
        .bind(("kind", kind.to_string()))
        .await
        .context(format!(
            "Failed to query default node_image from database: model={}, kind={}",
            model, kind
        ))?;

    let config: Option<NodeImageRow> = response.take(0)?;
    config.map(NodeConfig::try_from).transpose()
}

/// List all node_image records filtered by kind
#[instrument(skip(db), level = "debug")]
pub async fn list_node_images_by_kind(
    db: &Arc<Surreal<Client>>,
    kind: &NodeKind,
) -> Result<Vec<NodeConfig>> {
    let mut response = db
        .query("SELECT * FROM node_image WHERE kind = $kind ORDER BY model ASC")
        .bind(("kind", kind.to_string()))
        .await
        .context(format!(
            "Failed to query node_images by kind from database: kind={}",
            kind
        ))?;

    let configs: Vec<NodeImageRow> = response.take(0)?;
    configs.into_iter().map(NodeConfig::try_from).collect()
}

/// Get all versions of a node_image for a specific model and kind
#[instrument(skip(db), level = "debug")]
pub async fn get_node_image_versions(
    db: &Arc<Surreal<Client>>,
    model: &NodeModel,
    kind: &NodeKind,
) -> Result<Vec<NodeConfig>> {
    let mut response = db
        .query("SELECT * FROM node_image WHERE model = $model AND kind = $kind ORDER BY model ASC")
        .bind(("model", model.to_string()))
        .bind(("kind", kind.to_string()))
        .await
        .context(format!(
            "Failed to query node_image versions from database: model={}, kind={}",
            model, kind
        ))?;

    let configs: Vec<NodeImageRow> = response.take(0)?;
    configs.into_iter().map(NodeConfig::try_from).collect()
}

/// Get multiple node_images by a list of RecordIds in a single query
#[instrument(skip(db), level = "debug")]
pub async fn list_node_images_by_ids(
    db: &Arc<Surreal<Client>>,
    ids: Vec<RecordId>,
) -> Result<Vec<NodeConfig>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut response = db
        .query("SELECT * FROM $ids")
        .bind(("ids", ids.iter().map(to_surreal_id).collect::<Vec<_>>()))
        .await
        .context("Failed to batch query node_images by ids")?;

    let configs: Vec<NodeImageRow> = response.take(0)?;
    configs.into_iter().map(NodeConfig::try_from).collect()
}

/// Get node_image by RecordId
#[instrument(skip(db), level = "debug")]
pub async fn get_node_image_by_id(
    db: &Arc<Surreal<Client>>,
    id: RecordId,
) -> Result<Option<NodeConfig>> {
    let config: Option<NodeImageRow> = db
        .select(to_surreal_id(&id))
        .await
        .context(format!("Failed to query node_image by id: {:?}", id))?;

    config.map(NodeConfig::try_from).transpose()
}

/// Get node_image from node_model (returns error if not found)
/// This is used internally for image lookups by model.
#[allow(dead_code)]
#[instrument(skip(db), level = "debug")]
pub(crate) async fn get_node_image(
    db: &Arc<Surreal<Client>>,
    node_model: &NodeModel,
) -> Result<NodeConfig> {
    let mut response = db
        .query("SELECT * FROM ONLY node_image WHERE model = $model_id")
        .bind(("model_id", node_model.to_string()))
        .await
        .context(format!(
            "Failed to query node_image from database: {node_model}"
        ))?;

    let config: Option<NodeImageRow> = response.take(0)?;

    config
        .map(NodeConfig::try_from)
        .transpose()?
        .ok_or_else(|| anyhow!("Node image not found for model: {node_model}"))
}

/// Count total number of node_image records in the database
#[instrument(skip(db), level = "debug")]
pub async fn count_node_images(db: &Arc<Surreal<Client>>) -> Result<usize> {
    let configs: Vec<NodeImageRow> = db
        .select("node_image")
        .await
        .context("Failed to count node_images from database")?;

    Ok(configs.len())
}
