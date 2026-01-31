use anyhow::{Context, Result, anyhow};
use data::NodeConfig;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

/// Create a node_config record in the database
pub async fn create_node_config(db: &Surreal<Client>, config: NodeConfig) -> Result<NodeConfig> {
    let created_config: Option<NodeConfig> = db
        .create("node_config")
        .content(config.clone())
        .await
        .context(format!(
            "Error creating node_config (model and kind must be unique):\n model: {}\n kind: {}\n",
            config.model, config.kind
        ))?;

    created_config.ok_or_else(|| {
        anyhow!(
            "Node config was not created:\n model: {}\n kind: {}\n",
            config.model,
            config.kind
        )
    })
}

/// Upsert a node_config record (create if not exists, update if exists)
/// This uses a query-based approach to handle the unique constraint gracefully
pub async fn upsert_node_config(db: &Surreal<Client>, config: NodeConfig) -> Result<NodeConfig> {
    // Use SurrealDB's upsert method - this will create if not exists or update if exists
    let upserted_config: Option<NodeConfig> = db
        .upsert(("node_config", format!("{}_{}", config.model, config.kind)))
        .content(config.clone())
        .await
        .context(format!(
            "Error upserting node_config:\n model: {}\n kind: {}\n",
            config.model, config.kind
        ))?;

    upserted_config.ok_or_else(|| {
        anyhow!(
            "Node config was not upserted:\n model: {}\n kind: {}\n",
            config.model,
            config.kind
        )
    })
}
