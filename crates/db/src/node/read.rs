use anyhow::{Context, Result, anyhow};
use shared::data::{DbNode, RecordId};
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;
use tracing::instrument;

/// Get a node by its RecordId (surrogate key)
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The RecordId of the node
///
/// # Returns
/// The DbNode record
///
/// # Errors
/// - If node with id not found
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn get_node(db: &Arc<Surreal<Client>>, id: RecordId) -> Result<DbNode> {
    let node: Option<DbNode> = db
        .select(id.clone())
        .await
        .context(format!("Failed to get node by id: {:?}", id))?;

    node.ok_or_else(|| anyhow!("Node not found with id: {:?}", id))
}

/// Alias for get_node - kept for compatibility
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The RecordId of the node
///
/// # Returns
/// The DbNode record
///
/// # Errors
/// - If node with id not found
/// - If there's a database error
#[instrument(skip(db), level = "debug")]
pub async fn get_node_by_id(db: &Arc<Surreal<Client>>, id: RecordId) -> Result<DbNode> {
    get_node(db, id).await
}

/// Get a node by name and lab (unique constraint)
///
/// # Arguments
/// * `db` - Database connection
/// * `name` - Node name
/// * `lab_id` - Lab's RecordId
///
/// # Returns
/// The DbNode record
///
/// # Errors
/// - If node not found
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn get_node_by_name_and_lab(
    db: &Arc<Surreal<Client>>,
    name: &str,
    lab_id: RecordId,
) -> Result<DbNode> {
    let mut response = db
        .query("SELECT * FROM ONLY node WHERE name = $name AND lab = $lab_id")
        .bind(("name", name.to_string()))
        .bind(("lab_id", lab_id.clone()))
        .await
        .context(format!("Failed to query node by name and lab: {}", name))?;

    let node: Option<DbNode> = response.take(0)?;
    node.ok_or_else(|| anyhow!("Node not found with name '{}' in lab: {:?}", name, lab_id))
}

/// List all nodes in the database
///
/// # Arguments
/// * `db` - Database connection
///
/// # Returns
/// Vector of all DbNode records
///
/// # Errors
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn list_nodes(db: &Arc<Surreal<Client>>) -> Result<Vec<DbNode>> {
    let nodes: Vec<DbNode> = db
        .select("node")
        .await
        .context("Failed to list nodes from database")?;

    Ok(nodes)
}

/// List all nodes in a specific lab
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - Lab's RecordId
///
/// # Returns
/// Vector of DbNode records in the lab
///
/// # Errors
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn list_nodes_by_lab(db: &Arc<Surreal<Client>>, lab_id: RecordId) -> Result<Vec<DbNode>> {
    let mut response = db
        .query("SELECT * FROM node WHERE lab = $lab_id ORDER BY name ASC")
        .bind(("lab_id", lab_id.clone()))
        .await
        .context(format!("Failed to list nodes for lab: {:?}", lab_id))?;

    let nodes: Vec<DbNode> = response.take(0)?;
    Ok(nodes)
}

/// Count total number of nodes in the database
///
/// # Arguments
/// * `db` - Database connection
///
/// # Returns
/// Total count of nodes
///
/// # Errors
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn count_nodes(db: &Arc<Surreal<Client>>) -> Result<usize> {
    let mut response = db
        .query("SELECT count() FROM node GROUP ALL")
        .await
        .context("Failed to count nodes")?;

    let count: Option<usize> = response.take("count")?;
    Ok(count.unwrap_or(0))
}

/// Count number of nodes in a specific lab
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - Lab's RecordId
///
/// # Returns
/// Count of nodes in the lab
///
/// # Errors
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn count_nodes_by_lab(db: &Arc<Surreal<Client>>, lab_id: RecordId) -> Result<usize> {
    let mut response = db
        .query("SELECT count() FROM node WHERE lab = $lab_id GROUP ALL")
        .bind(("lab_id", lab_id))
        .await
        .context("Failed to count nodes for lab")?;

    let count: Option<usize> = response.take("count")?;
    Ok(count.unwrap_or(0))
}
