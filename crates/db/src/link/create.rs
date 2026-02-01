use anyhow::{Context, Result};
use data::{BridgeKind, DbLink, RecordId};
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

/// Create a new link between two nodes
///
/// This function creates a network link connecting two nodes in a lab.
/// Each link represents a virtual network cable with bridge and veth pair details.
///
/// # Arguments
/// * `db` - Database connection
/// * `index` - Link index (0-65535, unique per lab)
/// * `kind` - Bridge type (P2pBridge, P2pUdp, P2pVeth)
/// * `node_a_id` - RecordId of first node
/// * `node_b_id` - RecordId of second node
/// * `int_a` - Interface name on node_a
/// * `int_b` - Interface name on node_b
/// * `bridge_a` - Bridge name for node_a side
/// * `bridge_b` - Bridge name for node_b side
/// * `veth_a` - Virtual ethernet name for node_a side
/// * `veth_b` - Virtual ethernet name for node_b side
/// * `lab_id` - RecordId of the lab this link belongs to
///
/// # Returns
/// The created DbLink record with generated ID
///
/// # Errors
/// - If unique constraint is violated (node_a, node_b, int_a, int_b combination)
/// - If either node doesn't exist
/// - If lab doesn't exist
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, create_link};
/// # use data::BridgeKind;
/// # use surrealdb::RecordId;
/// # use konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let node_a_id: RecordId = ("node", "node1").into();
/// let node_b_id: RecordId = ("node", "node2").into();
/// let lab_id: RecordId = ("lab", "lab1").into();
///
/// let link = create_link(
///     &db,
///     0,
///     BridgeKind::P2pBridge,
///     node_a_id,
///     node_b_id,
///     "eth0".to_string(),
///     "eth0".to_string(),
///     "br0".to_string(),
///     "br1".to_string(),
///     "veth0".to_string(),
///     "veth1".to_string(),
///     lab_id,
/// ).await?;
/// # Ok(())
/// # }
/// ```
#[allow(clippy::too_many_arguments)]
pub async fn create_link(
    db: &Surreal<Client>,
    index: u16,
    kind: BridgeKind,
    node_a_id: RecordId,
    node_b_id: RecordId,
    int_a: String,
    int_b: String,
    bridge_a: String,
    bridge_b: String,
    veth_a: String,
    veth_b: String,
    lab_id: RecordId,
) -> Result<DbLink> {
    let link: Option<DbLink> = db
        .create("link")
        .content(DbLink {
            id: None,
            index,
            kind: kind.clone(),
            node_a: node_a_id.clone(),
            node_b: node_b_id.clone(),
            int_a: int_a.clone(),
            int_b: int_b.clone(),
            lab: lab_id.clone(),
            bridge_a: bridge_a.clone(),
            bridge_b: bridge_b.clone(),
            veth_a: veth_a.clone(),
            veth_b: veth_b.clone(),
        })
        .await
        .context(format!(
            "Failed to create link: index={}, node_a={}, node_b={}, int_a={}, int_b={}",
            index, node_a_id, node_b_id, int_a, int_b
        ))?;

    link.ok_or_else(|| {
        anyhow::anyhow!(
            "Link was not created: index={}, node_a={}, node_b={}, int_a={}, int_b={}",
            index,
            node_a_id,
            node_b_id,
            int_a,
            int_b
        )
    })
}
