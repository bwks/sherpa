use anyhow::{Context, Result, anyhow};
use shared::data::{DbNode, RecordId};
use tracing::instrument;
use turso::{Connection, Row};

use crate::Database;
use crate::persistence::{Cells, LAB_TABLE, NODE_TABLE, key_of, node_columns, node_from_row};
use crate::sql::{query_all, query_count, query_optional};

/// Read a node on `conn`, so writers can read under the write lock.
#[instrument(skip(conn), level = "debug")]
pub(super) async fn select_node(conn: &Connection, id: &RecordId) -> Result<DbNode> {
    query_optional(
        conn,
        concat!("SELECT ", node_columns!(), " FROM node WHERE id = ?1"),
        [key_of(id, NODE_TABLE)?],
        node_from_row,
    )
    .await
    .context(format!("Failed to get node by id: {:?}", id))?
    .ok_or_else(|| anyhow!("Node not found with id: {:?}", id))
}

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
pub async fn get_node(db: &Database, id: RecordId) -> Result<DbNode> {
    let conn = db.reader().await?;
    select_node(&conn, &id).await
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
pub async fn get_node_by_id(db: &Database, id: RecordId) -> Result<DbNode> {
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
    db: &Database,
    name: &str,
    lab_id: RecordId,
) -> Result<DbNode> {
    let conn = db.reader().await?;
    query_optional(
        &conn,
        concat!(
            "SELECT ",
            node_columns!(),
            " FROM node WHERE name = ?1 AND lab = ?2"
        ),
        [name.to_string(), key_of(&lab_id, LAB_TABLE)?],
        node_from_row,
    )
    .await
    .context(format!("Failed to query node by name and lab: {}", name))?
    .ok_or_else(|| anyhow!("Node not found with name '{}' in lab: {:?}", name, lab_id))
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
pub async fn list_nodes(db: &Database) -> Result<Vec<DbNode>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        concat!("SELECT ", node_columns!(), " FROM node"),
        (),
        node_from_row,
    )
    .await
    .context("Failed to list nodes from database")
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
pub async fn list_nodes_by_lab(db: &Database, lab_id: RecordId) -> Result<Vec<DbNode>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        concat!(
            "SELECT ",
            node_columns!(),
            " FROM node WHERE lab = ?1 ORDER BY name ASC"
        ),
        [key_of(&lab_id, LAB_TABLE)?],
        node_from_row,
    )
    .await
    .context(format!("Failed to list nodes for lab: {:?}", lab_id))
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
pub async fn count_nodes(db: &Database) -> Result<usize> {
    let conn = db.reader().await?;
    query_count(&conn, "SELECT count(*) FROM node", ())
        .await
        .context("Failed to count nodes")
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
pub async fn count_nodes_by_lab(db: &Database, lab_id: RecordId) -> Result<usize> {
    let conn = db.reader().await?;
    query_count(
        &conn,
        "SELECT count(*) FROM node WHERE lab = ?1",
        [key_of(&lab_id, LAB_TABLE)?],
    )
    .await
    .context("Failed to count nodes for lab")
}

/// Nodes with host CPUs pinned to their vCPUs or QEMU threads, in every lab.
#[instrument(skip(db), level = "debug")]
pub async fn list_pinned_nodes(db: &Database) -> Result<Vec<DbNode>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        concat!(
            "SELECT ",
            node_columns!(),
            " FROM node WHERE cpu_pinning IS NOT NULL OR qemu_thread_cpus IS NOT NULL"
        ),
        (),
        node_from_row,
    )
    .await
    .context("Failed to list nodes with pinned CPUs")
}

fn vnc_port_from_row(row: &Row) -> Result<u16> {
    Cells::new(row).integer("vnc_port")
}

/// The VNC ports held by nodes, in ascending order.
#[instrument(skip(db), level = "debug")]
pub async fn list_vnc_ports(db: &Database) -> Result<Vec<u16>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        "SELECT vnc_port FROM node WHERE vnc_port IS NOT NULL ORDER BY vnc_port ASC",
        (),
        vnc_port_from_row,
    )
    .await
    .context("Failed to list node VNC ports")
}
