use anyhow::{Context, Result, anyhow};
use shared::data::{DbNode, NodeState};
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;
use surrealdb_types::RecordId;

use crate::node::read::get_node;

/// Update an existing node in the database
///
/// **IMPORTANT:** The `lab` field is immutable and cannot be changed.
/// If the provided node has a different lab than the existing node, the update will fail.
/// Nodes cannot be moved between labs after creation.
///
/// # Arguments
/// * `db` - Database connection
/// * `node` - DbNode with all fields populated (id field is required)
///
/// # Returns
/// The updated DbNode record
///
/// # Errors
/// - If node.id is None (id is required for updates)
/// - If node doesn't exist
/// - If trying to change the lab (lab field is immutable)
/// - If unique constraints are violated (name+lab, index+lab)
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, get_node, update_node};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # use shared::data::RecordId;
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let id= RecordId::new("node", "abc123");
/// let mut node = get_node(&db, id).await?;
/// node.name = "updated-node".to_string();
/// let updated = update_node(&db, node).await?;
/// assert_eq!(updated.name, "updated-node");
/// # Ok(())
/// # }
/// ```
pub async fn update_node(db: &Arc<Surreal<Client>>, node: DbNode) -> Result<DbNode> {
    // Require id field for updates
    let id = node
        .id
        .as_ref()
        .ok_or_else(|| anyhow!("Cannot update node without id field"))?;

    // Verify the node exists and check if lab is being changed
    let existing = get_node(db, id.clone()).await?;

    // Verify lab is not being changed - it's immutable
    if existing.lab != node.lab {
        return Err(anyhow!(
            "Cannot change node lab: lab field is immutable. Nodes cannot be moved between labs. Existing lab: {:?}, attempted new lab: {:?}",
            existing.lab,
            node.lab
        ));
    }

    // Perform update
    let updated: Option<DbNode> = db
        .update(id.clone())
        .content(node.clone())
        .await
        .context(format!("Failed to update node: {}", node.name))?;

    updated.ok_or_else(|| anyhow!("Node update failed: {}", node.name))
}

/// Update a node's management IPv4 address.
///
/// Fetches the existing node, sets the `mgmt_ipv4` field, and writes it back.
///
/// # Arguments
/// * `db` - Database connection
/// * `node_id` - RecordId of the node to update
/// * `mgmt_ipv4` - The management IPv4 address to set
///
/// # Returns
/// The updated DbNode record
///
/// # Errors
/// - If the node doesn't exist
/// - If the database update fails
pub async fn update_node_mgmt_ipv4(
    db: &Arc<Surreal<Client>>,
    node_id: RecordId,
    mgmt_ipv4: &str,
) -> Result<DbNode> {
    let mut node = get_node(db, node_id.clone()).await?;
    node.mgmt_ipv4 = Some(mgmt_ipv4.to_string());

    let updated: Option<DbNode> = db
        .update(node_id.clone())
        .content(node.clone())
        .await
        .context(format!(
            "Failed to update mgmt_ipv4 for node: {}",
            node.name
        ))?;

    updated.ok_or_else(|| anyhow!("Node mgmt_ipv4 update failed: {}", node.name))
}

/// Update a node's runtime state.
///
/// Fetches the existing node, sets the `state` field, and writes it back.
///
/// # Arguments
/// * `db` - Database connection
/// * `node_id` - RecordId of the node to update
/// * `state` - The new NodeState to set
///
/// # Returns
/// The updated DbNode record
///
/// # Errors
/// - If the node doesn't exist
/// - If the database update fails
pub async fn update_node_state(
    db: &Arc<Surreal<Client>>,
    node_id: RecordId,
    state: NodeState,
) -> Result<DbNode> {
    let mut node = get_node(db, node_id.clone()).await?;
    node.state = state;

    let updated: Option<DbNode> = db
        .update(node_id.clone())
        .content(node.clone())
        .await
        .context(format!("Failed to update state for node: {}", node.name))?;

    updated.ok_or_else(|| anyhow!("Node state update failed: {}", node.name))
}
