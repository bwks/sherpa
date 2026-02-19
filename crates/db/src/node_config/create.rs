use std::sync::Arc;
use anyhow::{Context, Result, anyhow};
use shared::data::NodeConfig;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

use crate::node_config::get_node_config_by_model_kind_version;

/// Create a node_config record in the database with auto-generated ID
pub async fn create_node_config(db: &Arc<Surreal<Client>>, config: NodeConfig) -> Result<NodeConfig> {
    let created_config: Option<NodeConfig> = db
        .create("node_config")
        .content(config.clone())
        .await
        .context(format!(
            "Error creating node_config (model, kind, version must be unique):\n model: {}\n kind: {}\n version: {}\n",
            config.model, config.kind, config.version
        ))?;

    created_config.ok_or_else(|| {
        anyhow!(
            "Node config was not created:\n model: {}\n kind: {}\n version: {}\n",
            config.model,
            config.kind,
            config.version
        )
    })
}

/// Upsert a node_config record (create if not exists, update if exists)
/// This uses a query-based approach to handle the unique constraint gracefully.
/// If config.default is true, it will automatically unset default on other versions
/// of the same (model, kind) combination.
/// SurrealDB will auto-generate IDs for new records.
pub async fn upsert_node_config(db: &Arc<Surreal<Client>>, config: NodeConfig) -> Result<NodeConfig> {
    // If setting default=true, first unset default on other versions of same (model, kind)
    if config.default {
        db.query(
            "UPDATE node_config SET default = false 
             WHERE model = $model AND kind = $kind AND version != $version",
        )
        .bind(("model", config.model.to_string()))
        .bind(("kind", config.kind.to_string()))
        .bind(("version", config.version.clone()))
        .await
        .context(format!(
            "Error unsetting default flag on other versions:\n model: {}\n kind: {}\n",
            config.model, config.kind
        ))?;
    }

    // Try to find existing config by (model, kind, version) using the unique constraint
    let existing =
        get_node_config_by_model_kind_version(db, &config.model, &config.kind, &config.version)
            .await
            .context(format!(
                "Error querying existing node_config:\n model: {}\n kind: {}\n version: {}\n",
                config.model, config.kind, config.version
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
                "Error updating existing node_config:\n model: {}\n kind: {}\n version: {}\n",
                config.model, config.kind, config.version
            ))?;

        updated.ok_or_else(|| {
            anyhow!(
                "Node config was not updated:\n model: {}\n kind: {}\n version: {}\n",
                config.model,
                config.kind,
                config.version
            )
        })
    } else {
        // Create new record with auto-generated ID
        create_node_config(db, config).await
    }
}
