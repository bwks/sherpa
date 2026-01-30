use anyhow::Result;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

use data::{NodeConfig, NodeModel};

use super::upsert_node_config;

/// Delete all node_config records from the database
/// WARNING: This will remove all node configs including any custom ones
pub async fn delete_all_node_configs(db: &Surreal<Client>) -> Result<usize> {
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
    let models = NodeModel::variants();

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
                    println!("Created node config: {}", config.model);
                    created_count += 1;
                } else {
                    println!("Skipped node config (already exists): {}", config.model);
                    skipped_count += 1;
                }
            }
            Err(e) => {
                eprintln!("ERROR: Failed to upsert {}: {}", config.model, e);
                return Err(e.context(format!(
                    "Failed to seed node config: {} (created: {}, skipped: {})",
                    config.model, created_count, skipped_count
                )));
            }
        }
    }

    Ok(created_count)
}
