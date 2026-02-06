use anyhow::{Context, Result, anyhow};
use shared::data::DbNode;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

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
/// # use surrealdb::RecordId;
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let id: RecordId = ("node", "abc123").into();
/// let mut node = get_node(&db, id).await?;
/// node.name = "updated-node".to_string();
/// let updated = update_node(&db, node).await?;
/// assert_eq!(updated.name, "updated-node");
/// # Ok(())
/// # }
/// ```
pub async fn update_node(db: &Surreal<Client>, node: DbNode) -> Result<DbNode> {
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
            "Cannot change node lab: lab field is immutable. Nodes cannot be moved between labs. Existing lab: {}, attempted new lab: {}",
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
