use anyhow::{Context, Result, anyhow, bail};
use shared::data::{DbNode, NodeResources, NodeState, ReadyCheck, RecordId};
use tracing::instrument;
use turso::Value;

use crate::Database;
use crate::persistence::{NODE_IMAGE_TABLE, NODE_TABLE, enum_text, key_of, node_from_row};
use crate::queries;
use crate::sql::{Bound, query_count, query_optional};

/// Set columns of one node with `query`, whose `?1` is the node's key, and return the node.
async fn update_columns(
    db: &Database,
    node_id: &RecordId,
    query: Bound,
    what: &str,
) -> Result<DbNode> {
    let writer = db.writer().await?;
    query_optional(&writer.conn, query, node_from_row)
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
        queries::node::UPDATE_MGMT_IPV4
            .bind([key_of(&node_id, NODE_TABLE)?.into(), mgmt_ipv4.into()]),
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
        queries::node::UPDATE_MGMT_IPV6
            .bind([key_of(&node_id, NODE_TABLE)?.into(), mgmt_ipv6.into()]),
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
        queries::node::UPDATE_MGMT_MAC
            .bind([key_of(&node_id, NODE_TABLE)?.into(), mgmt_mac.into()]),
        "mgmt_mac",
    )
    .await
}

/// Move a node to state `to`, only if it is still in state `from`: a compare-and-set, so a
/// writer that read the state before another writer moved the node does not overwrite it.
/// Which moves are allowed is decided by `shared::data::NodeState::on_operation` and
/// `on_observation`; this only writes one.
///
/// # Returns
/// The updated node, or `None` when the node is no longer in state `from`.
///
/// # Errors
/// - If the node doesn't exist
/// - If the database update fails
#[instrument(skip(db), level = "debug")]
pub async fn update_node_state_from(
    db: &Database,
    node_id: RecordId,
    from: NodeState,
    to: NodeState,
) -> Result<Option<DbNode>> {
    let key = key_of(&node_id, NODE_TABLE)?;
    let writer = db.writer().await?;
    let node = query_optional(
        &writer.conn,
        queries::node::UPDATE_STATE_FROM.bind([
            Value::from(key.clone()),
            enum_text(&from, "state")?.into(),
            enum_text(&to, "state")?.into(),
        ]),
        node_from_row,
    )
    .await
    .context(format!("Failed to update state for node: {node_id:?}"))?;
    if node.is_none() {
        let exists = query_count(&writer.conn, queries::node::COUNT_BY_ID.bind([key.into()]))
            .await
            .context(format!("Failed to look up node: {node_id:?}"))?;
        if exists == 0 {
            bail!("Node not found with id: {node_id:?}");
        }
    }
    Ok(node)
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
        queries::node::UPDATE_SSH_HOST_KEY.bind([key_of(&node_id, NODE_TABLE)?.into(), key.into()]),
        "host key",
    )
    .await
}

/// Record what a node was given: its vCPUs, memory and VNC port. Its pinned host CPUs are in
/// the `cpu_pin` table.
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
        queries::node::UPDATE_RESOURCES.bind([
            key_of(&node_id, NODE_TABLE)?.into(),
            resources.cpu_count.into(),
            resources.memory.into(),
            resources.vnc_port.into(),
        ]),
        "resources",
    )
    .await
}

/// Point a node at the image it is built from, as a redeploy does when the manifest names
/// another version. The image must exist: `node.image` references `node_image`.
///
/// # Errors
/// - If the node doesn't exist
/// - If the image doesn't exist, or the database update fails
#[instrument(skip(db), level = "debug")]
pub async fn update_node_image_ref(
    db: &Database,
    node_id: RecordId,
    image_id: &RecordId,
) -> Result<DbNode> {
    update_columns(
        db,
        &node_id,
        queries::node::UPDATE_IMAGE.bind([
            key_of(&node_id, NODE_TABLE)?.into(),
            key_of(image_id, NODE_IMAGE_TABLE)?.into(),
        ]),
        "image",
    )
    .await
}

/// Record how a node shows it is ready: its ready port, whether its check is skipped and
/// whether its SSH server must show Sherpa's ready marker.
///
/// # Errors
/// - If the node doesn't exist
/// - If the database update fails
#[instrument(skip(db), level = "debug")]
pub async fn update_node_ready_check(
    db: &Database,
    node_id: RecordId,
    ready_check: &ReadyCheck,
) -> Result<DbNode> {
    update_columns(
        db,
        &node_id,
        queries::node::UPDATE_READY_CHECK.bind([
            key_of(&node_id, NODE_TABLE)?.into(),
            ready_check.port.into(),
            ready_check.skip.into(),
            ready_check.sshd_marker.into(),
        ]),
        "ready check",
    )
    .await
}

/// Set the account a node's SSH config logs in with: the lab owner's login, or `None` for the
/// shared `sherpa` account.
///
/// # Errors
/// - If the node doesn't exist
/// - If the database update fails
#[instrument(skip(db), level = "debug")]
pub async fn update_node_ssh_username(
    db: &Database,
    node_id: RecordId,
    ssh_username: Option<&str>,
) -> Result<DbNode> {
    update_columns(
        db,
        &node_id,
        queries::node::UPDATE_SSH_USERNAME
            .bind([key_of(&node_id, NODE_TABLE)?.into(), ssh_username.into()]),
        "ssh_username",
    )
    .await
}
