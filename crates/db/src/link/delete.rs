use anyhow::{Context, Result};
use data::{DbLink, RecordId};
use surrealdb::engine::remote::ws::Client;
use surrealdb::Surreal;

use crate::link::read::get_link;

/// Delete a link by its RecordId (surrogate key)
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The RecordId of the link
///
/// # Errors
/// - If link not found
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, delete_link};
/// # use surrealdb::RecordId;
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect("localhost", 8000, "test", "test").await?;
/// let id: RecordId = ("link", "abc123").into();
/// delete_link(&db, id).await?;
/// # Ok(())
/// # }
/// ```
pub async fn delete_link(db: &Surreal<Client>, id: RecordId) -> Result<()> {
    // Verify link exists
    let _ = get_link(db, id.clone()).await?;

    let _deleted: Option<DbLink> = db
        .delete(id.clone())
        .await
        .context(format!("Failed to delete link: {}", id))?;

    Ok(())
}

/// Alias for delete_link - kept for API consistency
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The RecordId of the link
///
/// # Errors
/// - If link not found
/// - If there's a database error
pub async fn delete_link_by_id(db: &Surreal<Client>, id: RecordId) -> Result<()> {
    delete_link(db, id).await
}

/// Delete all links for a lab
///
/// This is typically called when deleting a lab to clean up all associated links.
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - The RecordId of the lab
///
/// # Errors
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, delete_links_by_lab};
/// # use surrealdb::RecordId;
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect("localhost", 8000, "test", "test").await?;
/// let lab_id: RecordId = ("lab", "abc123").into();
/// delete_links_by_lab(&db, lab_id).await?;
/// # Ok(())
/// # }
/// ```
pub async fn delete_links_by_lab(db: &Surreal<Client>, lab_id: RecordId) -> Result<()> {
    let _deleted: Vec<DbLink> = db
        .query("DELETE link WHERE lab = $lab_id")
        .bind(("lab_id", lab_id.clone()))
        .await
        .context(format!("Failed to delete links for lab: {}", lab_id))?
        .take(0)?;

    Ok(())
}

/// Delete all links associated with a node
///
/// This deletes all links where the node appears as either node_a or node_b.
/// This is typically called before deleting a node to prevent foreign key violations.
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
/// # use db::{connect, delete_links_by_node};
/// # use surrealdb::RecordId;
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect("localhost", 8000, "test", "test").await?;
/// let node_id: RecordId = ("node", "abc123").into();
/// delete_links_by_node(&db, node_id).await?;
/// # Ok(())
/// # }
/// ```
pub async fn delete_links_by_node(db: &Surreal<Client>, node_id: RecordId) -> Result<()> {
    let _deleted: Vec<DbLink> = db
        .query("DELETE link WHERE node_a = $node_id OR node_b = $node_id")
        .bind(("node_id", node_id.clone()))
        .await
        .context(format!("Failed to delete links for node: {}", node_id))?
        .take(0)?;

    Ok(())
}
