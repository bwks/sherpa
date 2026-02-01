use anyhow::{Context, Result, anyhow};
use data::NodeConfig;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

use crate::helpers::get_config_id;

/// Update an existing node_config record in the database
///
/// This performs a full replacement of all fields. The NodeConfig must have
/// a valid `id` field set.
///
/// # Arguments
/// * `db` - Database connection
/// * `config` - NodeConfig with updated fields and a valid `id`
///
/// # Returns
/// The updated NodeConfig on success
///
/// # Errors
/// - If the config has no ID
/// - If the record doesn't exist in the database
/// - If there's a database error during the update
///
/// # Example
/// ```no_run
/// # use db::{connect, create_node_config, update_node_config};
/// # use konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # use data::{NodeModel, NodeConfig};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// 
/// // Create a config first
/// let test_config = NodeConfig::get_model(NodeModel::AristaVeos);
/// let created = create_node_config(&db, test_config).await?;
/// 
/// // Update it
/// let mut config = created.clone();
/// config.memory = 4096;  // Modify a field
/// let updated = update_node_config(&db, config).await?;
/// # Ok(())
/// # }
/// ```
pub async fn update_node_config(db: &Surreal<Client>, config: NodeConfig) -> Result<NodeConfig> {
    // Extract and validate the ID
    let id = get_config_id(&config)?;

    // Execute UPDATE query - replaces all fields
    let updated: Option<NodeConfig> = db
        .update(id.clone())
        .content(config.clone())
        .await
        .context(format!(
            "Failed to update node_config:\n id: {:?}\n model: {}\n kind: {}\n",
            id, config.model, config.kind
        ))?;

    // Return result or error if not found
    updated.ok_or_else(|| {
        anyhow!(
            "Node config not found for update:\n id: {:?}\n model: {}\n kind: {}\n",
            id,
            config.model,
            config.kind
        )
    })
}
