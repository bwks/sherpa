use anyhow::{Context, Result};
use shared::data::{BridgeMember, DbBridge, RecordId};
use tracing::instrument;

use crate::Database;
use crate::persistence::{
    bridge_columns, bridge_from_row, bridge_member_columns, bridge_member_values, bridge_values,
    new_key,
};
use crate::sql::query_optional;

/// Create a new shared bridge for a lab
///
/// This function creates a bridge record that represents a shared network
/// segment connecting multiple nodes. The bridge and its members are written in one
/// transaction.
///
/// # Arguments
/// * `db` - Database connection
/// * `index` - Bridge index (0-65535, unique per lab)
/// * `bridge_name` - Linux bridge name on the host
/// * `network_name` - Libvirt network name
/// * `lab_id` - RecordId of the lab this bridge belongs to
/// * `members` - The node interfaces connected to this bridge
///
/// # Returns
/// The created DbBridge record with generated ID
///
/// # Errors
/// - If unique constraint is violated (index, lab combination)
/// - If lab doesn't exist, or a member's node doesn't exist
/// - If there's a database error
#[instrument(skip(db), level = "debug")]
pub async fn create_bridge(
    db: &Database,
    index: u16,
    bridge_name: String,
    network_name: String,
    lab_id: RecordId,
    members: Vec<BridgeMember>,
) -> Result<DbBridge> {
    let domain = DbBridge {
        id: None,
        index,
        bridge_name: bridge_name.clone(),
        network_name,
        lab: lab_id.clone(),
        members,
    };
    let key = new_key();
    let failure = format!(
        "Failed to create bridge: index={}, bridge_name={}, lab_id={:?}",
        index, bridge_name, lab_id
    );

    let mut writer = db.writer().await?;
    let tx = writer.conn.transaction().await.context(failure.clone())?;

    let bridge = query_optional(
        &tx,
        concat!(
            "INSERT INTO bridge (",
            bridge_columns!(),
            ") VALUES (?1, ?2, ?3, ?4, ?5) RETURNING ",
            bridge_columns!()
        ),
        bridge_values(&domain, &key)?,
        bridge_from_row,
    )
    .await
    .context(failure.clone())?;

    let mut bridge = bridge.ok_or_else(|| {
        anyhow::anyhow!(
            "Bridge was not created: index={}, bridge_name={}, lab_id={:?}",
            index,
            bridge_name,
            lab_id
        )
    })?;

    for (position, member) in domain.members.iter().enumerate() {
        tx.execute(
            concat!(
                "INSERT INTO bridge_member (",
                bridge_member_columns!(),
                ") VALUES (?1, ?2, ?3, ?4)"
            ),
            bridge_member_values(member, &key, position)?,
        )
        .await
        .context(format!(
            "{failure}: member {} on node {:?}",
            member.interface, member.node
        ))?;
    }

    tx.commit().await.context(failure)?;

    bridge.members = domain.members;
    Ok(bridge)
}
