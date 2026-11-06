use anyhow::{Context, Result, anyhow};
use shared::data::{DbNode, RecordId};
use tracing::instrument;
use turso::{Connection, Row};

use crate::Database;
use crate::persistence::{Cells, LAB_TABLE, NODE_TABLE, key_of, node_from_row};
use crate::queries;
use crate::sql::{query_all, query_count, query_optional};

/// Get a node by name and lab (unique constraint)
///
/// # Arguments
/// * `db` - Database connection
/// * `name` - Node name
/// * `lab_id` - Lab's `RecordId`
///
/// # Returns
/// The `DbNode` record
///
/// # Errors
/// - If node not found
/// - If there's a database error
///
/// Read a node on `conn`, so writers can read under the write lock.
#[instrument(skip(conn), level = "debug")]
async fn select_node(conn: &Connection, id: &RecordId) -> Result<DbNode> {
    query_optional(
        conn,
        queries::node::FIND_BY_ID,
        [key_of(id, NODE_TABLE)?],
        node_from_row,
    )
    .await
    .context(format!("Failed to get node by id: {id:?}"))?
    .ok_or_else(|| anyhow!("Node not found with id: {id:?}"))
}

/// Get a node by its `RecordId` (surrogate key)
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The `RecordId` of the node
///
/// # Returns
/// The `DbNode` record
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

/// # Errors
///
/// Fails when there is no such node, or the database cannot be read.
#[instrument(skip(db), level = "debug")]
pub async fn get_node_by_name_and_lab(db: &Database, name: &str, lab: RecordId) -> Result<DbNode> {
    let conn = db.reader().await?;
    query_optional(
        &conn,
        queries::node::FIND_BY_NAME_AND_LAB,
        [name.to_string(), key_of(&lab, LAB_TABLE)?],
        node_from_row,
    )
    .await
    .context(format!("Failed to query node by name and lab: {name}"))?
    .ok_or_else(|| anyhow!("Node not found with name '{name}' in lab: {lab:?}"))
}

/// List all nodes in a specific lab
///
/// # Arguments
/// * `db` - Database connection
/// * `lab` - Lab's `RecordId`
///
/// # Returns
/// Vector of `DbNode` records in the lab
///
/// # Errors
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn list_nodes_by_lab(db: &Database, lab: RecordId) -> Result<Vec<DbNode>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        queries::node::LIST_BY_LAB,
        [key_of(&lab, LAB_TABLE)?],
        node_from_row,
    )
    .await
    .context(format!("Failed to list nodes for lab: {lab:?}"))
}

/// Count number of nodes in a specific lab
///
/// # Arguments
/// * `db` - Database connection
/// * `lab` - Lab's `RecordId`
///
/// # Returns
/// Count of nodes in the lab
///
/// # Errors
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn count_nodes_by_lab(db: &Database, lab: RecordId) -> Result<usize> {
    let conn = db.reader().await?;
    query_count(
        &conn,
        queries::node::COUNT_BY_LAB,
        [key_of(&lab, LAB_TABLE)?],
    )
    .await
    .context("Failed to count nodes for lab")
}

/// Nodes with host CPUs pinned to their vCPUs or QEMU threads, in every lab.
///
/// # Errors
///
/// Fails when the database cannot be read.
#[instrument(skip(db), level = "debug")]
pub async fn list_pinned_nodes(db: &Database) -> Result<Vec<DbNode>> {
    let conn = db.reader().await?;
    query_all(&conn, queries::node::LIST_PINNED, (), node_from_row)
        .await
        .context("Failed to list nodes with pinned CPUs")
}

fn vnc_port_from_row(row: &Row) -> Result<u16> {
    Cells::new(row).integer("vnc_port")
}

/// The VNC ports held by nodes, in ascending order.
///
/// # Errors
///
/// Fails when the database cannot be read.
#[instrument(skip(db), level = "debug")]
pub async fn list_vnc_ports(db: &Database) -> Result<Vec<u16>> {
    let conn = db.reader().await?;
    query_all(&conn, queries::node::VNC_PORTS, (), vnc_port_from_row)
        .await
        .context("Failed to list node VNC ports")
}
