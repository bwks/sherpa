use anyhow::{Context, Result, anyhow};
use shared::data::{DbLink, RecordId};
use tracing::instrument;

use crate::Database;
use crate::persistence::{LAB_TABLE, LINK_TABLE, NODE_TABLE, key_of, link_columns, link_from_row};
use crate::sql::{query_all, query_count, query_optional};

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
#[instrument(skip(db), level = "debug")]
pub async fn get_link(db: &Database, id: RecordId) -> Result<DbLink> {
    let conn = db.reader().await?;
    let link = query_optional(
        &conn,
        concat!("SELECT ", link_columns!(), " FROM link WHERE id = ?1"),
        [key_of(&id, LINK_TABLE)?],
        link_from_row,
    )
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
#[instrument(skip(db), level = "debug")]
pub async fn get_link_by_id(db: &Database, id: RecordId) -> Result<DbLink> {
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
#[instrument(skip(db), level = "debug")]
pub async fn get_link_by_peers(
    db: &Database,
    node_a_id: RecordId,
    node_b_id: RecordId,
    int_a: &str,
    int_b: &str,
) -> Result<DbLink> {
    let conn = db.reader().await?;
    let link = query_optional(
        &conn,
        concat!(
            "SELECT ",
            link_columns!(),
            " FROM link WHERE node_a = ?1 AND node_b = ?2 AND int_a = ?3 AND int_b = ?4"
        ),
        [
            key_of(&node_a_id, NODE_TABLE)?,
            key_of(&node_b_id, NODE_TABLE)?,
            int_a.to_string(),
            int_b.to_string(),
        ],
        link_from_row,
    )
    .await
    .context(format!(
        "Failed to query link by peers: node_a={:?}, node_b={:?}, int_a={}, int_b={}",
        node_a_id, node_b_id, int_a, int_b
    ))?;

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
#[instrument(skip(db), level = "debug")]
pub async fn list_links(db: &Database) -> Result<Vec<DbLink>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        concat!("SELECT ", link_columns!(), " FROM link"),
        (),
        link_from_row,
    )
    .await
    .context("Failed to list links from database")
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
#[instrument(skip(db), level = "debug")]
pub async fn list_links_by_lab(db: &Database, lab_id: RecordId) -> Result<Vec<DbLink>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        concat!("SELECT ", link_columns!(), " FROM link WHERE lab = ?1"),
        [key_of(&lab_id, LAB_TABLE)?],
        link_from_row,
    )
    .await
    .context(format!("Failed to list links for lab: {:?}", lab_id))
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
#[instrument(skip(db), level = "debug")]
pub async fn list_links_by_node(db: &Database, node_id: RecordId) -> Result<Vec<DbLink>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        concat!(
            "SELECT ",
            link_columns!(),
            " FROM link WHERE node_a = ?1 OR node_b = ?1"
        ),
        [key_of(&node_id, NODE_TABLE)?],
        link_from_row,
    )
    .await
    .context(format!("Failed to list links for node: {:?}", node_id))
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
#[instrument(skip(db), level = "debug")]
pub async fn count_links(db: &Database) -> Result<usize> {
    let conn = db.reader().await?;
    query_count(&conn, "SELECT count(*) FROM link", ())
        .await
        .context("Failed to count links")
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
#[instrument(skip(db), level = "debug")]
pub async fn count_links_by_lab(db: &Database, lab_id: RecordId) -> Result<usize> {
    let conn = db.reader().await?;
    query_count(
        &conn,
        "SELECT count(*) FROM link WHERE lab = ?1",
        [key_of(&lab_id, LAB_TABLE)?],
    )
    .await
    .context("Failed to count links for lab")
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
#[instrument(skip(db), level = "debug")]
pub async fn count_links_by_node(db: &Database, node_id: RecordId) -> Result<usize> {
    let conn = db.reader().await?;
    query_count(
        &conn,
        "SELECT count(*) FROM link WHERE node_a = ?1 OR node_b = ?1",
        [key_of(&node_id, NODE_TABLE)?],
    )
    .await
    .context("Failed to count links for node")
}
