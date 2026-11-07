use anyhow::{Context, Result, anyhow};
use shared::data::{DbNode, NodeResources, NodeState, RecordId};
use tracing::instrument;
use turso::Value;

use crate::Database;
use crate::persistence::{NODE_TABLE, enum_text, json_text, key_of, node_from_row};
use crate::queries;
use crate::sql::query_optional;

/// Set columns of one node with `sql`, whose `?1` is the node's key, and return the node.
async fn update_columns(
    db: &Database,
    node_id: &RecordId,
    sql: &str,
    mut values: Vec<Value>,
    what: &str,
) -> Result<DbNode> {
    values.insert(0, key_of(node_id, NODE_TABLE)?.into());
    let writer = db.writer().await?;
    query_optional(&writer.conn, sql, values, node_from_row)
        .await
        .context(format!("Failed to update {what} for node: {node_id:?}"))?
        .ok_or_else(|| anyhow!("Node not found with id: {node_id:?}"))
}

/// Update a node's management IPv4 address.
///
/// # Arguments
/// * `db` - Database connection
/// * `node_id` - `RecordId` of the node to update
/// * `mgmt_ipv4` - The management IPv4 address to set
///
/// # Returns
/// The updated `DbNode` record
///
/// # Errors
/// - If the node doesn't exist
/// - If the database update fails
#[instrument(skip(db), level = "debug")]
pub async fn update_node_mgmt_ipv4(
    db: &Database,
    node_id: RecordId,
    mgmt_ipv4: &str,
) -> Result<DbNode> {
    update_columns(
        db,
        &node_id,
        queries::node::UPDATE_MGMT_IPV4,
        vec![mgmt_ipv4.into()],
        "mgmt_ipv4",
    )
    .await
}

/// Update a node's management IPv6 address.
///
/// # Arguments
/// * `db` - Database connection
/// * `node_id` - `RecordId` of the node to update
/// * `mgmt_ipv6` - The management IPv6 address to set
///
/// # Returns
/// The updated `DbNode` record
///
/// # Errors
/// - If the node doesn't exist
/// - If the database update fails
#[instrument(skip(db), level = "debug")]
pub async fn update_node_mgmt_ipv6(
    db: &Database,
    node_id: RecordId,
    mgmt_ipv6: &str,
) -> Result<DbNode> {
    update_columns(
        db,
        &node_id,
        queries::node::UPDATE_MGMT_IPV6,
        vec![mgmt_ipv6.into()],
        "mgmt_ipv6",
    )
    .await
}

/// Update a node's management MAC address.
///
/// # Arguments
/// * `db` - Database connection
/// * `node_id` - `RecordId` of the node to update
/// * `mgmt_mac` - The management MAC address to set
///
/// # Returns
/// The updated `DbNode` record
///
/// # Errors
/// - If the node doesn't exist
/// - If the database update fails
#[instrument(skip(db), level = "debug")]
pub async fn update_node_mgmt_mac(
    db: &Database,
    node_id: RecordId,
    mgmt_mac: &str,
) -> Result<DbNode> {
    update_columns(
        db,
        &node_id,
        queries::node::UPDATE_MGMT_MAC,
        vec![mgmt_mac.into()],
        "mgmt_mac",
    )
    .await
}

/// Update a node's runtime state.
///
/// # Arguments
/// * `db` - Database connection
/// * `node_id` - `RecordId` of the node to update
/// * `state` - The new `NodeState` to set
///
/// # Returns
/// The updated `DbNode` record
///
/// # Errors
/// - If the node doesn't exist
/// - If the database update fails
#[instrument(skip(db), level = "debug")]
pub async fn update_node_state(
    db: &Database,
    node_id: RecordId,
    state: NodeState,
) -> Result<DbNode> {
    update_columns(
        db,
        &node_id,
        queries::node::UPDATE_STATE,
        vec![enum_text(&state, "state")?.into()],
        "state",
    )
    .await
}

/// Record the SSH host key a node offered, or clear it with `None`.
///
/// # Errors
/// - If the node doesn't exist
/// - If the database update fails
#[instrument(skip(db, key), level = "debug")]
pub async fn update_node_host_key(
    db: &Database,
    node_id: RecordId,
    key: Option<&str>,
) -> Result<DbNode> {
    update_columns(
        db,
        &node_id,
        queries::node::UPDATE_SSH_HOST_KEY,
        vec![key.into()],
        "host key",
    )
    .await
}

/// Record what a node was given: its vCPUs, memory, VNC port and pinned host CPUs.
///
/// # Errors
/// - If the node doesn't exist
/// - If another node already has the VNC port
/// - If the database update fails
#[instrument(skip(db), level = "debug")]
pub async fn update_node_resources(
    db: &Database,
    node_id: RecordId,
    resources: &NodeResources,
) -> Result<DbNode> {
    update_columns(
        db,
        &node_id,
        queries::node::UPDATE_RESOURCES,
        vec![
            resources.cpu_count.into(),
            resources.memory.into(),
            resources.vnc_port.into(),
            resources
                .cpu_pinning
                .as_ref()
                .map(|cpus| json_text(cpus, "cpu_pinning"))
                .transpose()?
                .into(),
            resources
                .qemu_thread_cpus
                .as_ref()
                .map(|cpus| json_text(cpus, "qemu_thread_cpus"))
                .transpose()?
                .into(),
        ],
        "resources",
    )
    .await
}
