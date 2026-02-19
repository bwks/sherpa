//! CREATE operations for nodes

use std::sync::Arc;
use anyhow::{Context, Result, anyhow};
use shared::data::DbNode;
use surrealdb::engine::remote::ws::Client;
use surrealdb::Surreal;
use surrealdb_types::RecordId;

/// Create a new node in the database.
///
/// Creates a node with the specified name, index, config reference, and lab reference.
/// The node name must be unique within the lab, and the index must also be unique within the lab.
///
/// # Parameters
///
/// * `db` - Database connection
/// * `name` - Node name (must be unique within the lab)
/// * `index` - Node index (must be unique within the lab, 0-65535)
/// * `config_id` - RecordId of the node_config template to use
/// * `lab_id` - RecordId of the lab this node belongs to
///
/// # Returns
///
/// * `Ok(DbNode)` - The created node with its assigned ID
/// * `Err` - If creation fails (e.g., duplicate name/index, invalid references)
///
/// # Examples
///
/// ```ignore
/// use db::{create_node, connect};
/// use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// use surrealdb::RecordId;
///
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let config_id: RecordId = ("node_config", "config1").into();
/// let lab_id: RecordId = ("lab", "lab1").into();
///
/// let node = create_node(&db, "router-1", 1, config_id, lab_id).await?;
/// println!("Created node: {}", node.name);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error if:
/// - A node with the same name already exists in the lab
/// - A node with the same index already exists in the lab
/// - The config_id doesn't reference a valid node_config
/// - The lab_id doesn't reference a valid lab
/// - Database operation fails
pub async fn create_node(
    db: &Arc<Surreal<Client>>,
    name: &str,
    index: u16,
    config_id: RecordId,
    lab_id: RecordId,
) -> Result<DbNode> {
    let node: Option<DbNode> = db
        .create("node")
        .content(DbNode {
            id: None,
            name: name.to_string(),
            config: config_id.clone(),
            index,
            lab: lab_id.clone(),
            mgmt_ipv4: None,
        })
        .await
        .context(format!(
            "Failed to create node: name='{}', index={}, lab_id={:?}",
            name, index, lab_id
        ))?;

    node.ok_or_else(|| {
        anyhow!(
            "Node was not created: name='{}', index={}, config_id={:?}, lab_id={:?}",
            name,
            index,
            config_id,
            lab_id
        )
    })
}
