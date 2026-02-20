use anyhow::{Context, Result, anyhow};
use shared::data::{NodeConfig, NodeKind, NodeModel, RecordId};
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

/// List all node_config records from the database ordered by model
pub async fn list_node_configs(db: &Arc<Surreal<Client>>) -> Result<Vec<NodeConfig>> {
    let mut response = db
        .query("SELECT * FROM node_config ORDER BY model ASC")
        .await
        .context("Failed to query all node_configs from database")?;

    let configs: Vec<NodeConfig> = response.take(0)?;
    Ok(configs)
}

/// Get node_config by model, kind, and version
pub async fn get_node_config_by_model_kind_version(
    db: &Arc<Surreal<Client>>,
    model: &NodeModel,
    kind: &NodeKind,
    version: &str,
) -> Result<Option<NodeConfig>> {
    let mut response = db
        .query("SELECT * FROM ONLY node_config WHERE model = $model AND kind = $kind AND version = $version")
        .bind(("model", model.to_string()))
        .bind(("kind", kind.to_string()))
        .bind(("version", version.to_string()))
        .await
        .context(format!(
            "Failed to query node_config from database: model={}, kind={}, version={}",
            model, kind, version
        ))?;

    let config: Option<NodeConfig> = response.take(0)?;
    Ok(config)
}

/// Get the default node_config for a specific model and kind
pub async fn get_default_node_config(
    db: &Arc<Surreal<Client>>,
    model: &NodeModel,
    kind: &NodeKind,
) -> Result<Option<NodeConfig>> {
    let mut response = db
        .query("SELECT * FROM ONLY node_config WHERE model = $model AND kind = $kind AND default = true")
        .bind(("model", model.to_string()))
        .bind(("kind", kind.to_string()))
        .await
        .context(format!(
            "Failed to query default node_config from database: model={}, kind={}",
            model, kind
        ))?;

    let config: Option<NodeConfig> = response.take(0)?;
    Ok(config)
}

/// List all node_config records filtered by kind
pub async fn list_node_configs_by_kind(
    db: &Arc<Surreal<Client>>,
    kind: &NodeKind,
) -> Result<Vec<NodeConfig>> {
    let mut response = db
        .query("SELECT * FROM node_config WHERE kind = $kind ORDER BY model ASC")
        .bind(("kind", kind.to_string()))
        .await
        .context(format!(
            "Failed to query node_configs by kind from database: kind={}",
            kind
        ))?;

    let configs: Vec<NodeConfig> = response.take(0)?;
    Ok(configs)
}

/// Get all versions of a node_config for a specific model and kind
pub async fn get_node_config_versions(
    db: &Arc<Surreal<Client>>,
    model: &NodeModel,
    kind: &NodeKind,
) -> Result<Vec<NodeConfig>> {
    let mut response = db
        .query("SELECT * FROM node_config WHERE model = $model AND kind = $kind ORDER BY model ASC")
        .bind(("model", model.to_string()))
        .bind(("kind", kind.to_string()))
        .await
        .context(format!(
            "Failed to query node_config versions from database: model={}, kind={}",
            model, kind
        ))?;

    let configs: Vec<NodeConfig> = response.take(0)?;
    Ok(configs)
}

/// Get node_config by RecordId
pub async fn get_node_config_by_id(
    db: &Arc<Surreal<Client>>,
    id: RecordId,
) -> Result<Option<NodeConfig>> {
    let config: Option<NodeConfig> = db
        .select(id.clone())
        .await
        .context(format!("Failed to query node_config by id: {:?}", id))?;

    Ok(config)
}

/// Get node_config from node_model (returns error if not found)
/// This is used internally for config lookups by model.
#[allow(dead_code)]
pub(crate) async fn get_node_config(
    db: &Arc<Surreal<Client>>,
    node_model: &NodeModel,
) -> Result<NodeConfig> {
    let mut response = db
        .query("SELECT * FROM ONLY node_config WHERE model = $model_id")
        .bind(("model_id", node_model.to_string()))
        .await
        .context(format!(
            "Failed to query node_config from database: {node_model}"
        ))?;

    let config: Option<NodeConfig> = response.take(0)?;

    config.ok_or_else(|| anyhow!("Node config not found for model: {node_model}"))
}

/// Count total number of node_config records in the database
pub async fn count_node_configs(db: &Arc<Surreal<Client>>) -> Result<usize> {
    let configs: Vec<NodeConfig> = db
        .select("node_config")
        .await
        .context("Failed to count node_configs from database")?;

    Ok(configs.len())
}
