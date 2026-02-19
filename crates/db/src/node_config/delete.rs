use anyhow::{Context, Result, anyhow};
use shared::data::{NodeConfig, RecordId};
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

/// Delete a node_config record from the database by its RecordId
///
/// Returns an error if:
/// - The record doesn't exist (returns "Config not found" error)
/// - The config is referenced by any nodes (database constraint will reject deletion)
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - RecordId of the node_config to delete
///
/// # Returns
/// `Ok(())` on successful deletion
///
/// # Errors
/// - If the record doesn't exist
/// - If the record is referenced by nodes (REFERENCE ON DELETE REJECT constraint)
/// - If there's a database error during deletion
///
/// # Example
/// ```no_run
/// # use db::{connect, create_node_config, delete_node_config};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # use shared::data::{NodeModel, NodeConfig};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
///
/// // Create a config first
/// let test_config = NodeConfig::get_model(NodeModel::AristaVeos);
/// let created = create_node_config(&db, test_config).await?;
/// let config_id = created.id.expect("Config should have ID");
///
/// // Delete it
/// delete_node_config(&db, config_id).await?;
/// # Ok(())
/// # }
/// ```
pub async fn delete_node_config(db: &Arc<Surreal<Client>>, id: RecordId) -> Result<()> {
    // Execute DELETE query
    let deleted: Option<NodeConfig> = db
        .delete(id.clone())
        .await
        .context(format!(
            "Failed to delete node_config: {:?}\nNote: Deletion will fail if any nodes reference this config",
            id
        ))?;

    // Verify the record was found and deleted
    deleted.ok_or_else(|| anyhow!("Node config not found for deletion: {:?}", id))?;

    Ok(())
}
