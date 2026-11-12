//! CREATE operations for nodes

use anyhow::{Context, Result, anyhow};
use shared::data::{DbNode, NodeState, RecordId};
use tracing::instrument;

use crate::Database;
use crate::persistence::{new_key, node_from_row, node_values};
use crate::queries;
use crate::sql::query_optional;

/// Create a new node in the database.
///
/// Creates a node with the specified name, index, image reference, and lab reference.
/// The node name must be unique within the lab, and the index must also be unique within the lab.
///
/// # Parameters
///
/// * `db` - Database connection
/// * `name` - Node name (must be unique within the lab)
/// * `index` - Node index (must be unique within the lab, 0-65535)
/// * `image_id` - `RecordId` of the `node_image` to use
/// * `lab` - `RecordId` of the lab this node belongs to
///
/// # Returns
///
/// * `Ok(DbNode)` - The created node with its assigned ID
/// * `Err` - If creation fails (e.g., duplicate name/index, invalid references)
///
/// # Errors
///
/// Returns an error if:
/// - A node with the same name already exists in the lab
/// - A node with the same index already exists in the lab
/// - The `image_id` doesn't reference a valid `node_image`
/// - The lab doesn't reference a valid lab
/// - Database operation fails
#[instrument(skip(db), level = "debug")]
pub async fn create_node(
    db: &Database,
    name: &str,
    index: u16,
    image_id: RecordId,
    lab: RecordId,
) -> Result<DbNode> {
    let domain = DbNode {
        id: None,
        name: name.to_string(),
        image: image_id.clone(),
        index,
        lab: lab.clone(),
        mgmt_ipv4: None,
        mgmt_ipv6: None,
        mgmt_mac: None,
        state: NodeState::Building,
        cpu_count: None,
        memory: None,
        vnc_port: None,
        ssh_host_key: None,
        cpu_pinning: None,
        qemu_thread_cpus: None,
    };

    let writer = db.writer().await?;
    let node = query_optional(
        &writer.conn,
        queries::node::CREATE.bind(node_values(&domain, &new_key())?),
        node_from_row,
    )
    .await
    .context(format!(
        "Failed to create node: name='{name}', index={index}, lab={lab:?}"
    ))?;

    node.ok_or_else(|| {
        anyhow!(
            "Node was not created: name='{name}', index={index}, image_id={image_id:?}, lab={lab:?}"
        )
    })
}
