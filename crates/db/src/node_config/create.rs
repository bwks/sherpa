use anyhow::{Context, Result, anyhow};
use data::NodeConfig;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

use crate::node_config::get_node_config_by_model_kind;

/// Create a node_config record in the database with auto-generated ID
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
/// This uses a query-based approach to handle the unique constraint gracefully.
/// SurrealDB will auto-generate IDs for new records.
pub async fn upsert_node_config(db: &Surreal<Client>, config: NodeConfig) -> Result<NodeConfig> {
    // First, try to find existing config by model + kind using the unique constraint
    let existing = get_node_config_by_model_kind(db, &config.model, &config.kind.to_string())
        .await
        .context(format!(
            "Error querying existing node_config:\n model: {}\n kind: {}\n",
            config.model, config.kind
        ))?;

    if let Some(existing_config) = existing {
        // Update existing record, preserving the auto-generated ID
        let mut updated_config = config.clone();
        updated_config.id = existing_config.id.clone();
        
        let updated: Option<NodeConfig> = db
            .update(existing_config.id.unwrap())
            .content(updated_config)
            .await
            .context(format!(
                "Error updating existing node_config:\n model: {}\n kind: {}\n",
                config.model, config.kind
            ))?;

        updated.ok_or_else(|| {
            anyhow!(
                "Node config was not updated:\n model: {}\n kind: {}\n",
                config.model,
                config.kind
            )
        })
    } else {
        // Create new record with auto-generated ID
        create_node_config(db, config).await
    }
}
