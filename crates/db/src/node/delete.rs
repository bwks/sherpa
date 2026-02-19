use std::sync::Arc;
use anyhow::{Context, Result, anyhow};
use shared::data::{DbLink, DbNode, RecordId};
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

use crate::node::read::get_node;

/// Delete a node by its RecordId (surrogate key)
///
/// **WARNING:** This function only deletes the node record itself.
/// If the node has associated links, this will fail due to foreign key constraints.
///
/// For explicit control over deletion order, use `delete_node_cascade()`.
/// To check for dependencies before deletion, use `delete_node_safe()`.
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The RecordId of the node
///
/// # Errors
/// - If node not found
/// - If node has associated links
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, delete_node};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # use surrealdb::RecordId;
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let id: RecordId = ("node", "abc123").into();
/// delete_node(&db, id).await?;
/// # Ok(())
/// # }
/// ```
pub async fn delete_node(db: &Arc<Surreal<Client>>, id: RecordId) -> Result<()> {
    // Verify node exists
    let _ = get_node(db, id.clone()).await?;

    let _deleted: Option<DbNode> = db
        .delete(id.clone())
        .await
        .context(format!("Failed to delete node: {:?}", id))?;

    Ok(())
}

/// Alias for delete_node - kept for API consistency
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The RecordId of the node
///
/// # Errors
/// - If node not found
/// - If node has associated links
/// - If there's a database error
pub async fn delete_node_by_id(db: &Arc<Surreal<Client>>, id: RecordId) -> Result<()> {
    delete_node(db, id).await
}

/// Delete all nodes for a lab
///
/// **WARNING:** This function deletes all nodes in a lab.
/// If any node has associated links, this will fail due to foreign key constraints.
/// Use `delete_lab_cascade()` from the lab module for proper cleanup.
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - The RecordId of the lab
///
/// # Errors
/// - If lab not found
/// - If any node has associated links
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, delete_nodes_by_lab};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # use surrealdb::RecordId;
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let lab_id: RecordId = ("lab", "abc123").into();
/// delete_nodes_by_lab(&db, lab_id).await?;
/// # Ok(())
/// # }
/// ```
pub async fn delete_nodes_by_lab(db: &Arc<Surreal<Client>>, lab_id: RecordId) -> Result<()> {
    let _deleted: Vec<DbNode> = db
        .query("DELETE node WHERE lab = $lab_id")
        .bind(("lab_id", lab_id.clone()))
        .await
        .context(format!("Failed to delete nodes for lab: {:?}", lab_id))?
        .take(0)?;

    Ok(())
}

/// Delete all links associated with a node
///
/// This is a helper function that deletes all links where the node
/// appears as either node_a or node_b. This should be called before
/// deleting a node to prevent foreign key constraint violations.
///
/// # Arguments
/// * `db` - Database connection
/// * `node_id` - The RecordId of the node
///
/// # Errors
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, delete_node_links};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # use surrealdb::RecordId;
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let node_id: RecordId = ("node", "abc123").into();
/// delete_node_links(&db, node_id).await?;
/// # Ok(())
/// # }
/// ```
pub async fn delete_node_links(db: &Arc<Surreal<Client>>, node_id: RecordId) -> Result<()> {
    let _deleted: Vec<DbLink> = db
        .query("DELETE link WHERE node_a = $node_id OR node_b = $node_id")
        .bind(("node_id", node_id.clone()))
        .await
        .context(format!("Failed to delete links for node: {:?}", node_id))?
        .take(0)?;

    Ok(())
}

/// Delete a node with explicit cascade (delete links, then node)
///
/// This function explicitly deletes all dependencies in the correct order:
/// 1. Delete all links where this node appears (as node_a or node_b)
/// 2. Delete the node
///
/// This provides explicit control over the deletion order and ensures
/// no foreign key constraint violations occur.
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The RecordId of the node
///
/// # Errors
/// - If node not found
/// - If there's a database error during any deletion step
///
/// # Example
/// ```no_run
/// # use db::{connect, delete_node_cascade};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # use surrealdb::RecordId;
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let id: RecordId = ("node", "abc123").into();
/// delete_node_cascade(&db, id).await?;
/// println!("Node and all its links deleted");
/// # Ok(())
/// # }
/// ```
pub async fn delete_node_cascade(db: &Arc<Surreal<Client>>, id: RecordId) -> Result<()> {
    // Delete in order: links -> node
    delete_node_links(db, id.clone()).await?;
    delete_node(db, id).await?;

    Ok(())
}

/// Delete a node safely (only if it has no links)
///
/// This function checks if the node has any associated links before deletion.
/// If links exist, it returns an error with details about what's blocking deletion.
///
/// Use this when you want to prevent accidental deletion of nodes that still have
/// active network connections.
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The RecordId of the node
///
/// # Errors
/// - If node not found
/// - If node has links (won't delete)
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, delete_node_safe};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # use surrealdb::RecordId;
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let id: RecordId = ("node", "abc123").into();
///
/// match delete_node_safe(&db, id).await {
///     Ok(_) => println!("Node deleted successfully"),
///     Err(e) => println!("Cannot delete node: {}", e),
/// }
/// # Ok(())
/// # }
/// ```
pub async fn delete_node_safe(db: &Arc<Surreal<Client>>, id: RecordId) -> Result<()> {
    // Get the node to verify it exists
    let node = get_node(db, id.clone()).await?;

    // Check for links (where node appears as either node_a or node_b)
    let links: Vec<DbLink> = db
        .query("SELECT * FROM link WHERE node_a = $node_id OR node_b = $node_id")
        .bind(("node_id", id.clone()))
        .await
        .context("Failed to check for links")?
        .take(0)?;

    if !links.is_empty() {
        return Err(anyhow!(
            "Cannot delete node '{}' ({:?}): node has {} associated link(s). Delete links first or use delete_node_cascade()",
            node.name,
            id,
            links.len()
        ));
    }

    // Safe to delete - node has no links
    delete_node(db, id).await
}
