use anyhow::{Context, Result, anyhow};
use shared::data::{DbNode, NodeResources, NodeState, RecordId};
use tracing::instrument;
use turso::Value;

use crate::Database;
use crate::node::read::select_node;
use crate::persistence::{
    NODE_TABLE, enum_text, json_text, key_of, node_columns, node_from_row, node_values,
};
use crate::sql::query_optional;

/// Update an existing node in the database
///
/// **IMPORTANT:** The `lab` field is immutable and cannot be changed.
/// If the provided node has a different lab than the existing node, the update will fail.
/// Nodes cannot be moved between labs after creation.
///
/// # Arguments
/// * `db` - Database connection
/// * `node` - DbNode with all fields populated (id field is required)
///
/// # Returns
/// The updated DbNode record
///
/// # Errors
/// - If node.id is None (id is required for updates)
/// - If node doesn't exist
/// - If trying to change the lab (lab field is immutable)
/// - If unique constraints are violated (name+lab, index+lab)
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn update_node(db: &Database, node: DbNode) -> Result<DbNode> {
    // Require id field for updates
    let id = node
        .id
        .as_ref()
        .ok_or_else(|| anyhow!("Cannot update node without id field"))?;
    let key = key_of(id, NODE_TABLE)?;

    // Read and write under one write lock, so the lab check holds for the update.
    let writer = db.writer().await?;
    let existing = select_node(&writer.conn, id).await?;

    // Verify lab is not being changed - it's immutable
    if existing.lab != node.lab {
        return Err(anyhow!(
            "Cannot change node lab: lab field is immutable. Nodes cannot be moved between labs. Existing lab: {:?}, attempted new lab: {:?}",
            existing.lab,
            node.lab
        ));
    }

    query_optional(
        &writer.conn,
        concat!(
            "UPDATE node SET name = ?2, \"index\" = ?3, image = ?4, lab = ?5, mgmt_ipv4 = ?6, \
             mgmt_ipv6 = ?7, mgmt_mac = ?8, state = ?9, cpu_count = ?10, memory = ?11, \
             vnc_port = ?12, ssh_host_key = ?13, cpu_pinning = ?14, qemu_thread_cpus = ?15 \
             WHERE id = ?1 RETURNING ",
            node_columns!()
        ),
        node_values(&node, &key)?,
        node_from_row,
    )
    .await
    .context(format!("Failed to update node: {}", node.name))?
    .ok_or_else(|| anyhow!("Node update failed: {}", node.name))
}

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
        .context(format!("Failed to update {what} for node: {:?}", node_id))?
        .ok_or_else(|| anyhow!("Node not found with id: {:?}", node_id))
}

/// Update a node's management IPv4 address.
///
/// # Arguments
/// * `db` - Database connection
/// * `node_id` - RecordId of the node to update
/// * `mgmt_ipv4` - The management IPv4 address to set
///
/// # Returns
/// The updated DbNode record
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
        concat!(
            "UPDATE node SET mgmt_ipv4 = ?2 WHERE id = ?1 RETURNING ",
            node_columns!()
        ),
        vec![mgmt_ipv4.into()],
        "mgmt_ipv4",
    )
    .await
}

/// Update a node's management IPv6 address.
///
/// # Arguments
/// * `db` - Database connection
/// * `node_id` - RecordId of the node to update
/// * `mgmt_ipv6` - The management IPv6 address to set
///
/// # Returns
/// The updated DbNode record
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
        concat!(
            "UPDATE node SET mgmt_ipv6 = ?2 WHERE id = ?1 RETURNING ",
            node_columns!()
        ),
        vec![mgmt_ipv6.into()],
        "mgmt_ipv6",
    )
    .await
}

/// Update a node's management MAC address.
///
/// # Arguments
/// * `db` - Database connection
/// * `node_id` - RecordId of the node to update
/// * `mgmt_mac` - The management MAC address to set
///
/// # Returns
/// The updated DbNode record
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
        concat!(
            "UPDATE node SET mgmt_mac = ?2 WHERE id = ?1 RETURNING ",
            node_columns!()
        ),
        vec![mgmt_mac.into()],
        "mgmt_mac",
    )
    .await
}

/// Update a node's runtime state.
///
/// # Arguments
/// * `db` - Database connection
/// * `node_id` - RecordId of the node to update
/// * `state` - The new NodeState to set
///
/// # Returns
/// The updated DbNode record
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
        concat!(
            "UPDATE node SET state = ?2 WHERE id = ?1 RETURNING ",
            node_columns!()
        ),
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
        concat!(
            "UPDATE node SET ssh_host_key = ?2 WHERE id = ?1 RETURNING ",
            node_columns!()
        ),
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
        concat!(
            "UPDATE node SET cpu_count = ?2, memory = ?3, vnc_port = ?4, cpu_pinning = ?5, \
             qemu_thread_cpus = ?6 WHERE id = ?1 RETURNING ",
            node_columns!()
        ),
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
