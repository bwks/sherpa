use anyhow::{Context, Result, anyhow};
use shared::data::{NodeConfig, NodeKind, NodeModel, RecordId};
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

/// List all node_image records from the database ordered by model
pub async fn list_node_images(db: &Arc<Surreal<Client>>) -> Result<Vec<NodeConfig>> {
    let mut response = db
        .query("SELECT * FROM node_image ORDER BY model ASC")
        .await
        .context("Failed to query all node_images from database")?;

    let configs: Vec<NodeConfig> = response.take(0)?;
    Ok(configs)
}

/// Get node_image by model, kind, and version
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

    let config: Option<NodeConfig> = response.take(0)?;
    Ok(config)
}

/// Get the default node_image for a specific model and kind
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

    let config: Option<NodeConfig> = response.take(0)?;
    Ok(config)
}

/// List all node_image records filtered by kind
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

    let configs: Vec<NodeConfig> = response.take(0)?;
    Ok(configs)
}

/// Get all versions of a node_image for a specific model and kind
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

    let configs: Vec<NodeConfig> = response.take(0)?;
    Ok(configs)
}

/// Get node_image by RecordId
pub async fn get_node_image_by_id(
    db: &Arc<Surreal<Client>>,
    id: RecordId,
) -> Result<Option<NodeConfig>> {
    let config: Option<NodeConfig> = db
        .select(id.clone())
        .await
        .context(format!("Failed to query node_image by id: {:?}", id))?;

    Ok(config)
}

/// Get node_image from node_model (returns error if not found)
/// This is used internally for image lookups by model.
#[allow(dead_code)]
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

    let config: Option<NodeConfig> = response.take(0)?;

    config.ok_or_else(|| anyhow!("Node image not found for model: {node_model}"))
}

/// Count total number of node_image records in the database
pub async fn count_node_images(db: &Arc<Surreal<Client>>) -> Result<usize> {
    let configs: Vec<NodeConfig> = db
        .select("node_image")
        .await
        .context("Failed to count node_images from database")?;

    Ok(configs.len())
}
