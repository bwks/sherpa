use anyhow::{Context, Result};
use shared::data::{BridgeKind, DbLink, LinkImpairment, RecordId};
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;
use tracing::instrument;

use crate::persistence::LinkRow;

/// Create a new link between two nodes
///
/// This function creates a network link connecting two nodes in a lab.
/// Each link represents a virtual network cable between two tap or veth host ends.
///
/// # Arguments
/// * `db` - Database connection
/// * `index` - Link index (0-65535, unique per lab)
/// * `kind` - Bridge type (P2p, SharedBridge)
/// * `node_a_id` - RecordId of first node
/// * `node_b_id` - RecordId of second node
/// * `int_a` - Interface name on node_a
/// * `int_b` - Interface name on node_b
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
    tap_a: String,
    tap_b: String,
    lab_id: RecordId,
) -> Result<DbLink> {
    let domain = DbLink {
        id: None,
        index,
        kind: kind.clone(),
        node_a: node_a_id.clone(),
        node_b: node_b_id.clone(),
        int_a: int_a.clone(),
        int_b: int_b.clone(),
        lab: lab_id.clone(),
        tap_a: tap_a.clone(),
        tap_b: tap_b.clone(),
        impairment_a_to_b: LinkImpairment::default(),
        impairment_b_to_a: LinkImpairment::default(),
    };
    let link: Option<LinkRow> = db
        .create("link")
        .content(LinkRow::try_from(&domain)?)
        .await
        .context(format!(
            "Failed to create link: index={}, node_a={:?}, node_b={:?}, int_a={}, int_b={}",
            index, node_a_id, node_b_id, int_a, int_b
        ))?;

    link.map(DbLink::try_from).transpose()?.ok_or_else(|| {
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
