use anyhow::{Context, Result};
use shared::data::{DbNode, NodeState, RecordId};
use tracing::instrument;
use turso::Connection;

use crate::persistence::{Cells, LAB_TABLE, NODE_TABLE, USER_TABLE, key_of, node_from_row};
use crate::queries;
use crate::sql::{query_all, query_count, query_optional};
use crate::{Database, DbError};

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
        queries::node::FIND_BY_ID.bind([key_of(id, NODE_TABLE)?.into()]),
        node_from_row,
    )
    .await
    .context(format!("Failed to get node by id: {id:?}"))?
    .ok_or_else(|| DbError::NodeNotFound { id: id.clone() }.into())
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
        queries::node::FIND_BY_NAME_AND_LAB
            .bind([name.to_string().into(), key_of(&lab, LAB_TABLE)?.into()]),
        node_from_row,
    )
    .await
    .context(format!("Failed to query node by name and lab: {name}"))?
    .ok_or_else(|| {
        DbError::NodeNotFoundInLab {
            name: name.to_owned(),
            lab,
        }
        .into()
    })
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
        queries::node::LIST_BY_LAB.bind([key_of(&lab, LAB_TABLE)?.into()]),
        node_from_row,
    )
    .await
    .context(format!("Failed to list nodes for lab: {lab:?}"))
}

/// The state of every node in `user`'s labs, each with its lab's `RecordId`, in one query. Lab
/// statuses are derived from these when the labs are listed.
///
/// # Errors
/// - If there's a database error
#[instrument(skip(db), level = "debug")]
pub async fn list_node_states_by_user(
    db: &Database,
    user: RecordId,
) -> Result<Vec<(RecordId, NodeState)>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        queries::node::LIST_STATES_BY_USER.bind([key_of(&user, USER_TABLE)?.into()]),
        |row| {
            let mut cells = Cells::new(row);
            Ok((cells.record(LAB_TABLE, "lab")?, cells.enumeration("state")?))
        },
    )
    .await
    .context(format!("Failed to list the node states of user {user:?}"))
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
        queries::node::COUNT_BY_LAB.bind([key_of(&lab, LAB_TABLE)?.into()]),
    )
    .await
    .context("Failed to count nodes for lab")
}

/// The number of nodes, in every lab, whose recorded SSH login is `username`.
///
/// # Errors
///
/// Fails when the database cannot be read.
#[instrument(skip(db), level = "debug")]
pub async fn count_nodes_by_ssh_username(db: &Database, username: &str) -> Result<usize> {
    let conn = db.reader().await?;
    query_count(
        &conn,
        queries::node::COUNT_BY_SSH_USERNAME.bind([username.into()]),
    )
    .await
    .context("Failed to count nodes by SSH login")
}
