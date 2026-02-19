use std::sync::Arc;
use anyhow::{Context, Result, anyhow};
use shared::data::{DbLink, RecordId};
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

/// Get a link by its RecordId (surrogate key)
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The RecordId of the link
///
/// # Returns
/// The DbLink record
///
/// # Errors
/// - If link with id not found
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, get_link};
/// # use surrealdb::RecordId;
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let id: RecordId = ("link", "abc123").into();
/// let link = get_link(&db, id).await?;
/// # Ok(())
/// # }
/// ```
pub async fn get_link(db: &Arc<Surreal<Client>>, id: RecordId) -> Result<DbLink> {
    let link: Option<DbLink> = db
        .select(id.clone())
        .await
        .context(format!("Failed to get link by id: {:?}", id))?;

    link.ok_or_else(|| anyhow!("Link not found with id: {:?}", id))
}

/// Alias for get_link - kept for compatibility
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The RecordId of the link
///
/// # Returns
/// The DbLink record
///
/// # Errors
/// - If link with id not found
/// - If there's a database error
pub async fn get_link_by_id(db: &Arc<Surreal<Client>>, id: RecordId) -> Result<DbLink> {
    get_link(db, id).await
}

/// Get a link by its peer nodes and interfaces (unique constraint)
///
/// # Arguments
/// * `db` - Database connection
/// * `node_a_id` - RecordId of first node
/// * `node_b_id` - RecordId of second node
/// * `int_a` - Interface name on node_a
/// * `int_b` - Interface name on node_b
///
/// # Returns
/// The DbLink record
///
/// # Errors
/// - If link not found
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, get_link_by_peers};
/// # use surrealdb::RecordId;
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let node_a_id: RecordId = ("node", "node1").into();
/// let node_b_id: RecordId = ("node", "node2").into();
/// let link = get_link_by_peers(&db, node_a_id, node_b_id, "eth0", "eth0").await?;
/// # Ok(())
/// # }
/// ```
pub async fn get_link_by_peers(
    db: &Arc<Surreal<Client>>,
    node_a_id: RecordId,
    node_b_id: RecordId,
    int_a: &str,
    int_b: &str,
) -> Result<DbLink> {
    let mut response = db
        .query(
            "SELECT * FROM ONLY link WHERE node_a = $node_a AND node_b = $node_b AND int_a = $int_a AND int_b = $int_b",
        )
        .bind(("node_a", node_a_id.clone()))
        .bind(("node_b", node_b_id.clone()))
        .bind(("int_a", int_a.to_string()))
        .bind(("int_b", int_b.to_string()))
        .await
        .context(format!(
            "Failed to query link by peers: node_a={:?}, node_b={:?}, int_a={}, int_b={}",
            node_a_id, node_b_id, int_a, int_b
        ))?;

    let link: Option<DbLink> = response.take(0)?;
    link.ok_or_else(|| {
        anyhow!(
            "Link not found with peers: node_a={:?}, node_b={:?}, int_a={}, int_b={}",
            node_a_id,
            node_b_id,
            int_a,
            int_b
        )
    })
}

/// List all links in the database
///
/// # Arguments
/// * `db` - Database connection
///
/// # Returns
/// Vector of all DbLink records
///
/// # Errors
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, list_links};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let links = list_links(&db).await?;
/// println!("Found {} links", links.len());
/// # Ok(())
/// # }
/// ```
pub async fn list_links(db: &Arc<Surreal<Client>>) -> Result<Vec<DbLink>> {
    let links: Vec<DbLink> = db
        .select("link")
        .await
        .context("Failed to list links from database")?;

    Ok(links)
}

/// List all links in a specific lab
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - Lab's RecordId
///
/// # Returns
/// Vector of DbLink records in the lab
///
/// # Errors
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, list_links_by_lab};
/// # use surrealdb::RecordId;
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let lab_id: RecordId = ("lab", "lab1").into();
/// let links = list_links_by_lab(&db, lab_id).await?;
/// println!("Lab contains {} links", links.len());
/// # Ok(())
/// # }
/// ```
pub async fn list_links_by_lab(db: &Arc<Surreal<Client>>, lab_id: RecordId) -> Result<Vec<DbLink>> {
    let mut response = db
        .query("SELECT * FROM link WHERE lab = $lab_id")
        .bind(("lab_id", lab_id.clone()))
        .await
        .context(format!("Failed to list links for lab: {:?}", lab_id))?;

    let links: Vec<DbLink> = response.take(0)?;
    Ok(links)
}

/// List all links associated with a specific node
///
/// Returns links where the node appears as either node_a or node_b.
///
/// # Arguments
/// * `db` - Database connection
/// * `node_id` - Node's RecordId
///
/// # Returns
/// Vector of DbLink records connected to the node
///
/// # Errors
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, list_links_by_node};
/// # use surrealdb::RecordId;
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let node_id: RecordId = ("node", "node1").into();
/// let links = list_links_by_node(&db, node_id).await?;
/// println!("Node has {} links", links.len());
/// # Ok(())
/// # }
/// ```
pub async fn list_links_by_node(db: &Arc<Surreal<Client>>, node_id: RecordId) -> Result<Vec<DbLink>> {
    let mut response = db
        .query("SELECT * FROM link WHERE node_a = $node_id OR node_b = $node_id")
        .bind(("node_id", node_id.clone()))
        .await
        .context(format!("Failed to list links for node: {:?}", node_id))?;

    let links: Vec<DbLink> = response.take(0)?;
    Ok(links)
}

/// Count total number of links in the database
///
/// # Arguments
/// * `db` - Database connection
///
/// # Returns
/// Total count of links
///
/// # Errors
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, count_links};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let count = count_links(&db).await?;
/// println!("Total links: {}", count);
/// # Ok(())
/// # }
/// ```
pub async fn count_links(db: &Arc<Surreal<Client>>) -> Result<usize> {
    let mut response = db
        .query("SELECT count() FROM link GROUP ALL")
        .await
        .context("Failed to count links")?;

    let count: Option<usize> = response.take("count")?;
    Ok(count.unwrap_or(0))
}

/// Count number of links in a specific lab
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - Lab's RecordId
///
/// # Returns
/// Count of links in the lab
///
/// # Errors
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, count_links_by_lab};
/// # use surrealdb::RecordId;
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let lab_id: RecordId = ("lab", "lab1").into();
/// let count = count_links_by_lab(&db, lab_id).await?;
/// println!("Lab contains {} links", count);
/// # Ok(())
/// # }
/// ```
pub async fn count_links_by_lab(db: &Arc<Surreal<Client>>, lab_id: RecordId) -> Result<usize> {
    let mut response = db
        .query("SELECT count() FROM link WHERE lab = $lab_id GROUP ALL")
        .bind(("lab_id", lab_id))
        .await
        .context("Failed to count links for lab")?;

    let count: Option<usize> = response.take("count")?;
    Ok(count.unwrap_or(0))
}

/// Count number of links associated with a specific node
///
/// Returns count of links where the node appears as either node_a or node_b.
///
/// # Arguments
/// * `db` - Database connection
/// * `node_id` - Node's RecordId
///
/// # Returns
/// Count of links connected to the node
///
/// # Errors
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, count_links_by_node};
/// # use surrealdb::RecordId;
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let node_id: RecordId = ("node", "node1").into();
/// let count = count_links_by_node(&db, node_id).await?;
/// println!("Node has {} links", count);
/// # Ok(())
/// # }
/// ```
pub async fn count_links_by_node(db: &Arc<Surreal<Client>>, node_id: RecordId) -> Result<usize> {
    let mut response = db
        .query("SELECT count() FROM link WHERE node_a = $node_id OR node_b = $node_id GROUP ALL")
        .bind(("node_id", node_id))
        .await
        .context("Failed to count links for node")?;

    let count: Option<usize> = response.take("count")?;
    Ok(count.unwrap_or(0))
}
