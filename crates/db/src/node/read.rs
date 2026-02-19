use std::sync::Arc;
use anyhow::{Context, Result, anyhow};
use shared::data::{DbNode, RecordId};
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

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
/// # Example
/// ```no_run
/// # use db::{connect, get_node};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # use surrealdb::RecordId;
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let id: RecordId = ("node", "abc123").into();
/// let node = get_node(&db, id).await?;
/// # Ok(())
/// # }
/// ```
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
/// # Example
/// ```no_run
/// # use db::{connect, get_node_by_name_and_lab, create_lab, create_user};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let user = create_user(&db, "alice".to_string(), vec![]).await?;
/// let lab = create_lab(&db, "My Lab", "lab-001", &user).await?;
/// let lab_id = lab.id.unwrap();
/// let node = get_node_by_name_and_lab(&db, "node1", lab_id).await?;
/// # Ok(())
/// # }
/// ```
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
/// # Example
/// ```no_run
/// # use db::{connect, list_nodes};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let nodes = list_nodes(&db).await?;
/// println!("Found {} nodes", nodes.len());
/// # Ok(())
/// # }
/// ```
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
/// # Example
/// ```no_run
/// # use db::{connect, list_nodes_by_lab, create_lab, create_user};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let user = create_user(&db, "alice".to_string(), vec![]).await?;
/// let lab = create_lab(&db, "My Lab", "lab-001", &user).await?;
/// let lab_id = lab.id.unwrap();
/// let nodes = list_nodes_by_lab(&db, lab_id).await?;
/// println!("Lab contains {} nodes", nodes.len());
/// # Ok(())
/// # }
/// ```
pub async fn list_nodes_by_lab(db: &Arc<Surreal<Client>>, lab_id: RecordId) -> Result<Vec<DbNode>> {
    let mut response = db
        .query("SELECT * FROM node WHERE lab = $lab_id")
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
/// # Example
/// ```no_run
/// # use db::{connect, count_nodes};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let count = count_nodes(&db).await?;
/// println!("Total nodes: {}", count);
/// # Ok(())
/// # }
/// ```
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
/// # Example
/// ```no_run
/// # use db::{connect, count_nodes_by_lab, create_lab, create_user};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let user = create_user(&db, "alice".to_string(), vec![]).await?;
/// let lab = create_lab(&db, "My Lab", "lab-001", &user).await?;
/// let lab_id = lab.id.unwrap();
/// let count = count_nodes_by_lab(&db, lab_id).await?;
/// println!("Lab contains {} nodes", count);
/// # Ok(())
/// # }
/// ```
pub async fn count_nodes_by_lab(db: &Arc<Surreal<Client>>, lab_id: RecordId) -> Result<usize> {
    let mut response = db
        .query("SELECT count() FROM node WHERE lab = $lab_id GROUP ALL")
        .bind(("lab_id", lab_id))
        .await
        .context("Failed to count nodes for lab")?;

    let count: Option<usize> = response.take("count")?;
    Ok(count.unwrap_or(0))
}
