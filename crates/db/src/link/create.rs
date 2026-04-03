use anyhow::{Context, Result};
use shared::data::{BridgeKind, DbLink, RecordId};
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;
use tracing::instrument;

/// Create a new link between two nodes
///
/// This function creates a network link connecting two nodes in a lab.
/// Each link represents a virtual network cable with bridge/veth/tap details.
///
/// # Arguments
/// * `db` - Database connection
/// * `index` - Link index (0-65535, unique per lab)
/// * `kind` - Bridge type (P2p, P2pBridge, P2pUdp, P2pVeth)
/// * `node_a_id` - RecordId of first node
/// * `node_b_id` - RecordId of second node
/// * `int_a` - Interface name on node_a
/// * `int_b` - Interface name on node_b
/// * `bridge_a` - Bridge name for node_a side
/// * `bridge_b` - Bridge name for node_b side
/// * `veth_a` - Virtual ethernet name for node_a side
/// * `veth_b` - Virtual ethernet name for node_b side
/// * `tap_a` - Tap device name for node_a side (P2p links)
/// * `tap_b` - Tap device name for node_b side (P2p links)
/// * `lab_id` - RecordId of the lab this link belongs to
///
/// # Returns
/// The created DbLink record with generated ID
#[allow(clippy::too_many_arguments)]
#[instrument(skip(db), level = "debug")]
pub async fn create_link(
    db: &Arc<Surreal<Client>>,
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
    tap_a: String,
    tap_b: String,
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
            tap_a: tap_a.clone(),
            tap_b: tap_b.clone(),
            delay_us: 0,
            jitter_us: 0,
            loss_percent: 0.0,
            reorder_percent: 0.0,
            corrupt_percent: 0.0,
        })
        .await
        .context(format!(
            "Failed to create link: index={}, node_a={:?}, node_b={:?}, int_a={}, int_b={}",
            index, node_a_id, node_b_id, int_a, int_b
        ))?;

    link.ok_or_else(|| {
        anyhow::anyhow!(
            "Link was not created: index={}, node_a={:?}, node_b={:?}, int_a={}, int_b={}",
            index,
            node_a_id,
            node_b_id,
            int_a,
            int_b
        )
    })
}
