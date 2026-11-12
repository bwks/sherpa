use anyhow::{Context, Result};
use shared::data::{BridgeMember, DbBridge, RecordId};
use tracing::instrument;
use turso::Connection;

use crate::Database;
use crate::persistence::{
    BRIDGE_TABLE, LAB_TABLE, bridge_from_row, bridge_member_from_row, key_of,
};
use crate::queries;
use crate::sql::query_all;

/// Load a bridge's members, in order, into `bridge`.
async fn with_members(conn: &Connection, mut bridge: DbBridge) -> Result<DbBridge> {
    let id = bridge.id.as_ref().context("Bridge record has no ID")?;
    let members: Vec<BridgeMember> = query_all(
        conn,
        queries::bridge::LIST_MEMBERS.bind([key_of(id, BRIDGE_TABLE)?.into()]),
        bridge_member_from_row,
    )
    .await
    .context(format!("Failed to get members of bridge: id={id:?}"))?;
    bridge.members = members;
    Ok(bridge)
}

/// List all bridges for a lab
///
/// # Arguments
/// * `db` - Database connection
/// * `lab` - `RecordId` of the lab
///
/// # Returns
/// Vector of `DbBridge` records
///
/// # Errors
/// - If there's a database error
#[instrument(skip(db), level = "debug")]
pub async fn list_bridges(db: &Database, lab: &RecordId) -> Result<Vec<DbBridge>> {
    let conn = db.reader().await?;
    let bridges = query_all(
        &conn,
        queries::bridge::LIST_BY_LAB.bind([key_of(lab, LAB_TABLE)?.into()]),
        bridge_from_row,
    )
    .await
    .context(format!("Failed to list bridges for lab: lab={lab:?}"))?;

    let mut loaded = Vec::with_capacity(bridges.len());
    for bridge in bridges {
        loaded.push(with_members(&conn, bridge).await?);
    }
    Ok(loaded)
}
