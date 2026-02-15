use anyhow::Result;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

use shared::data::{NodeConfig, NodeModel};

use crate::node_config::upsert_node_config;

/// Delete all node_config records from the database
/// WARNING: This will remove all node configs including any custom ones
pub async fn delete_node_configs(db: &Surreal<Client>) -> Result<usize> {
    let deleted: Vec<NodeConfig> = db.query("DELETE node_config").await?.take(0)?;

    Ok(deleted.len())
}

/// Seed the database with all default node configurations
///
/// This function inserts all pre-configured NodeConfig structs into the database.
/// Uses upsert logic: if a config already exists, it's returned unchanged; otherwise it's created.
///
/// Returns the number of configs successfully created (not including pre-existing ones).
pub async fn seed_node_configs(db: &Surreal<Client>) -> Result<usize> {
    let models = NodeModel::to_vec();

    let mut created_count = 0;
    let mut skipped_count = 0;

    for model in models {
        let config = NodeConfig::get_model(model);

        // Get current count to detect if this is a new insert
        let before_count = db.select::<Vec<NodeConfig>>("node_config").await?.len();

        match upsert_node_config(db, config.clone()).await {
            Ok(_) => {
                let after_count = db.select::<Vec<NodeConfig>>("node_config").await?.len();

                if after_count > before_count {
                    tracing::debug!(model = %config.model, "Created node config");
                    created_count += 1;
                } else {
                    tracing::debug!(model = %config.model, "Skipped node config (already exists)");
                    skipped_count += 1;
                }
            }
            Err(e) => {
                tracing::error!(model = %config.model, error = %e, "Failed to create node config");
                return Err(e.context(format!(
                    "Failed to seed node config: {} (created: {}, skipped: {})",
                    config.model, created_count, skipped_count
                )));
            }
        }
    }

    Ok(created_count)
}
