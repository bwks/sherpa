use anyhow::{Context, Result};
use shared::data::{BridgeKind, DbLink, LinkImpairment, RecordId};
use tracing::instrument;

use crate::Database;
use crate::persistence::{link_columns, link_from_row, link_values, new_key};
use crate::sql::query_optional;

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
    db: &Database,
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
        kind,
        node_a: node_a_id.clone(),
        node_b: node_b_id.clone(),
        int_a: int_a.clone(),
        int_b: int_b.clone(),
        lab: lab_id,
        tap_a,
        tap_b,
        impairment_a_to_b: LinkImpairment::default(),
        impairment_b_to_a: LinkImpairment::default(),
    };

    let writer = db.writer().await?;
    let link = query_optional(
        &writer.conn,
        concat!(
            "INSERT INTO link (",
            link_columns!(),
            ") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
             ?17, ?18, ?19, ?20) RETURNING ",
            link_columns!()
        ),
        link_values(&domain, &new_key())?,
        link_from_row,
    )
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
