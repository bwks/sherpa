use anyhow::{Context, Result};
use shared::data::{BridgeMember, DbBridge, RecordId};
use tracing::instrument;
use turso::Connection;

use crate::Database;
use crate::persistence::{
    BRIDGE_TABLE, LAB_TABLE, bridge_columns, bridge_from_row, bridge_member_from_row, key_of,
};
use crate::sql::{query_all, query_optional};

/// Load a bridge's members, in order, into `bridge`.
async fn with_members(conn: &Connection, mut bridge: DbBridge) -> Result<DbBridge> {
    let id = bridge.id.as_ref().context("Bridge record has no ID")?;
    let members: Vec<BridgeMember> = query_all(
        conn,
        "SELECT node, interface FROM bridge_member WHERE bridge = ?1 ORDER BY position",
        [key_of(id, BRIDGE_TABLE)?],
        bridge_member_from_row,
    )
    .await
    .context(format!("Failed to get members of bridge: id={:?}", id))?;
    bridge.members = members;
    Ok(bridge)
}

/// Get a bridge by its ID
///
/// # Arguments
/// * `db` - Database connection
/// * `bridge_id` - RecordId of the bridge
///
/// # Returns
/// The DbBridge record if found
///
/// # Errors
/// - If the bridge doesn't exist
/// - If there's a database error
#[instrument(skip(db), level = "debug")]
pub async fn get_bridge(db: &Database, bridge_id: &RecordId) -> Result<DbBridge> {
    let conn = db.reader().await?;
    let bridge = query_optional(
        &conn,
        concat!("SELECT ", bridge_columns!(), " FROM bridge WHERE id = ?1"),
        [key_of(bridge_id, BRIDGE_TABLE)?],
        bridge_from_row,
    )
    .await
    .context(format!("Failed to get bridge: id={:?}", bridge_id))?
    .ok_or_else(|| anyhow::anyhow!("Bridge not found: id={:?}", bridge_id))?;

    with_members(&conn, bridge).await
}

/// Get a bridge by its index and lab
///
/// # Arguments
/// * `db` - Database connection
/// * `index` - Bridge index
/// * `lab_id` - RecordId of the lab
///
/// # Returns
/// The DbBridge record if found
///
/// # Errors
/// - If the bridge doesn't exist
/// - If there's a database error
#[instrument(skip(db), level = "debug")]
pub async fn get_bridge_by_index(db: &Database, index: u16, lab_id: &RecordId) -> Result<DbBridge> {
    let conn = db.reader().await?;
    let bridge = query_optional(
        &conn,
        concat!(
            "SELECT ",
            bridge_columns!(),
            " FROM bridge WHERE \"index\" = ?1 AND lab = ?2"
        ),
        (index, key_of(lab_id, LAB_TABLE)?),
        bridge_from_row,
    )
    .await
    .context(format!(
        "Failed to get bridge by index: index={}, lab_id={:?}",
        index, lab_id
    ))?
    .ok_or_else(|| anyhow::anyhow!("Bridge not found: index={}, lab_id={:?}", index, lab_id))?;

    with_members(&conn, bridge).await
}

/// List all bridges for a lab
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - RecordId of the lab
///
/// # Returns
/// Vector of DbBridge records
///
/// # Errors
/// - If there's a database error
#[instrument(skip(db), level = "debug")]
pub async fn list_bridges(db: &Database, lab_id: &RecordId) -> Result<Vec<DbBridge>> {
    let conn = db.reader().await?;
    let bridges = query_all(
        &conn,
        concat!(
            "SELECT ",
            bridge_columns!(),
            " FROM bridge WHERE lab = ?1 ORDER BY \"index\""
        ),
        [key_of(lab_id, LAB_TABLE)?],
        bridge_from_row,
    )
    .await
    .context(format!(
        "Failed to list bridges for lab: lab_id={:?}",
        lab_id
    ))?;

    let mut loaded = Vec::with_capacity(bridges.len());
    for bridge in bridges {
        loaded.push(with_members(&conn, bridge).await?);
    }
    Ok(loaded)
}
