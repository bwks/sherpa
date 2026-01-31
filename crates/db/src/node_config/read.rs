use anyhow::{anyhow, Context, Result};
use data::{NodeConfig, NodeModel, RecordId};
use surrealdb::engine::remote::ws::Client;
use surrealdb::Surreal;

/// List all node_config records from the database
pub async fn list_node_configs(db: &Surreal<Client>) -> Result<Vec<NodeConfig>> {
    let configs: Vec<NodeConfig> = db
        .select("node_config")
        .await
        .context("Failed to query all node_configs from database")?;

    Ok(configs)
}

/// Get node_config by model and kind
pub async fn get_node_config_by_model_kind(
    db: &Surreal<Client>,
    model: &NodeModel,
    kind: &str,
) -> Result<Option<NodeConfig>> {
    let mut response = db
        .query("SELECT * FROM ONLY node_config WHERE model = $model AND kind = $kind")
        .bind(("model", model.to_string()))
        .bind(("kind", kind.to_string()))
        .await
        .context(format!(
            "Failed to query node_config from database: model={}, kind={}",
            model, kind
        ))?;

    let config: Option<NodeConfig> = response.take(0)?;
    Ok(config)
}

/// Get node_config by RecordId
pub async fn get_node_config_by_id(
    db: &Surreal<Client>,
    id: RecordId,
) -> Result<Option<NodeConfig>> {
    let config: Option<NodeConfig> = db
        .select(id.clone())
        .await
        .context(format!("Failed to query node_config by id: {:?}", id))?;

    Ok(config)
}

/// Get node_config from node_model (returns error if not found)
/// This is used internally by create_lab_node in action.rs
pub(crate) async fn get_node_config(
    db: &Surreal<Client>,
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
pub async fn count_node_configs(db: &Surreal<Client>) -> Result<usize> {
    let configs: Vec<NodeConfig> = db
        .select("node_config")
        .await
        .context("Failed to count node_configs from database")?;

    Ok(configs.len())
}
