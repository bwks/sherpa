//! Writing the records read from SurrealDB into the embedded database.
use anyhow::{Context, Result, bail};
use shared::data::RecordId;
use tracing::instrument;
use turso::Connection;

use super::source::{SurrealData, SurrealSource, connect, read_all};
use crate::Database;
use crate::persistence::{
    BRIDGE_TABLE, LAB_TABLE, LINK_TABLE, NODE_IMAGE_TABLE, NODE_TABLE, USER_TABLE,
    bridge_member_values, bridge_values, key_of, lab_values, link_values, node_image_values,
    node_values, user_values,
};
use crate::queries;
use crate::schema::{FIX_DATA_INTERFACE_COUNTS, apply_schema};
use crate::sql::query_count;

/// How many records of each kind were imported.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub users: usize,
    pub node_images: usize,
    pub labs: usize,
    pub nodes: usize,
    pub links: usize,
    pub bridges: usize,
    pub bridge_members: usize,
}

impl ImportSummary {
    fn of(data: &SurrealData) -> Self {
        Self {
            users: data.users.len(),
            node_images: data.node_images.len(),
            labs: data.labs.len(),
            nodes: data.nodes.len(),
            links: data.links.len(),
            bridges: data.bridges.len(),
            bridge_members: data.bridges.iter().map(|bridge| bridge.members.len()).sum(),
        }
    }
}

/// Copy every record of a pre-0.5.0 server's SurrealDB into `target`, keeping record keys.
///
/// The schema is applied to `target` first. The copy runs in one transaction: if anything
/// fails, `target` is left as it was. SurrealDB is only read.
///
/// # Errors
///
/// - If SurrealDB cannot be reached, signed in to or read, or holds a record that does not
///   convert to Sherpa's records
/// - If `target` already holds a user or a lab, so an import never mixes with existing data
/// - If a record violates the schema, for example a node whose lab does not exist
/// - If the number of rows written for a table differs from the number read
///
#[instrument(skip(target), fields(host = %source.host, port = source.port))]
pub async fn import_from_surrealdb(
    source: &SurrealSource,
    target: &Database,
) -> Result<ImportSummary> {
    let surreal = connect(source).await?;
    let data = read_all(&surreal).await?;
    let expected = ImportSummary::of(&data);
    tracing::info!(?expected, "Read SurrealDB records");

    apply_schema(target)
        .await
        .context("Failed to prepare the database for the import")?;

    let mut writer = target.writer().await?;
    let tx = writer
        .conn
        .transaction()
        .await
        .context("Failed to start the import transaction")?;
    match write_all(&tx, &data, &expected).await {
        Ok(()) => {
            tx.commit()
                .await
                .context("Failed to commit the import transaction")?;
            tracing::info!(?expected, "Imported SurrealDB records");
            Ok(expected)
        }
        Err(error) => {
            if let Err(rollback) = tx.rollback().await {
                tracing::error!(error = ?rollback, "Failed to roll back the import transaction");
            }
            Err(error)
        }
    }
}

async fn write_all(conn: &Connection, data: &SurrealData, expected: &ImportSummary) -> Result<()> {
    let users = query_count(conn, queries::user::COUNT, ()).await?;
    let labs = query_count(conn, queries::lab::COUNT, ()).await?;
    if users != 0 || labs != 0 {
        bail!(
            "The database already holds {users} users and {labs} labs; the import only writes \
             into an empty database"
        );
    }

    // Parents before children, so every foreign key finds its row.
    for user in &data.users {
        let key = record_key(user.id.as_ref(), USER_TABLE)?;
        conn.execute(queries::user::IMPORT, user_values(user, &key)?)
            .await
            .context(format!("Failed to import user {}", user.username))?;
    }
    for image in &data.node_images {
        let key = record_key(image.id.as_ref(), NODE_IMAGE_TABLE)?;
        conn.execute(queries::node_image::IMPORT, node_image_values(image, &key)?)
            .await
            .context(format!(
                "Failed to import node image {:?} {}",
                image.model, image.version
            ))?;
    }
    // The schema was brought up to date before the rows existed, so its corrections did not see
    // them.
    conn.execute_batch(FIX_DATA_INTERFACE_COUNTS)
        .await
        .context("Failed to correct imported node image interface counts")?;
    for lab in &data.labs {
        let key = record_key(lab.id.as_ref(), LAB_TABLE)?;
        conn.execute(queries::lab::IMPORT, lab_values(lab, &key)?)
            .await
            .context(format!(
                "Failed to import lab {} ({})",
                lab.name, lab.lab_id
            ))?;
    }
    for node in &data.nodes {
        let key = record_key(node.id.as_ref(), NODE_TABLE)?;
        conn.execute(queries::node::IMPORT, node_values(node, &key)?)
            .await
            .context(format!("Failed to import node {}", node.name))?;
    }
    for link in &data.links {
        let key = record_key(link.id.as_ref(), LINK_TABLE)?;
        conn.execute(queries::link::IMPORT, link_values(link, &key)?)
            .await
            .context(format!(
                "Failed to import link {} {} - {} {}",
                link.node_a, link.int_a, link.node_b, link.int_b
            ))?;
    }
    for bridge in &data.bridges {
        let key = record_key(bridge.id.as_ref(), BRIDGE_TABLE)?;
        conn.execute(queries::bridge::IMPORT, bridge_values(bridge, &key)?)
            .await
            .context(format!("Failed to import bridge {}", bridge.bridge_name))?;
        for (position, member) in bridge.members.iter().enumerate() {
            conn.execute(
                queries::bridge::ADD_MEMBER,
                bridge_member_values(member, &key, position)?,
            )
            .await
            .context(format!(
                "Failed to import bridge {} member {} {}",
                bridge.bridge_name, member.node, member.interface
            ))?;
        }
    }

    let written = ImportSummary {
        users: query_count(conn, queries::user::COUNT, ()).await?,
        node_images: query_count(conn, queries::node_image::COUNT, ()).await?,
        labs: query_count(conn, queries::lab::COUNT, ()).await?,
        nodes: query_count(conn, queries::node::COUNT, ()).await?,
        links: query_count(conn, queries::link::COUNT, ()).await?,
        bridges: query_count(conn, queries::bridge::COUNT, ()).await?,
        bridge_members: query_count(conn, queries::bridge::COUNT_MEMBERS, ()).await?,
    };
    if written != *expected {
        bail!("Import row counts differ: read {expected:?}, wrote {written:?}");
    }
    Ok(())
}

/// The key of a record read from SurrealDB, which always has an id.
fn record_key(id: Option<&RecordId>, table: &str) -> Result<String> {
    match id {
        Some(id) => key_of(id, table),
        None => bail!("A SurrealDB {table} record has no id"),
    }
}

#[cfg(test)]
mod tests {
    use shared::data::{BridgeMember, DbBridge};

    use super::*;

    #[test]
    fn summary_counts_bridge_members() {
        let member = BridgeMember {
            node: RecordId::new("node", "n1"),
            interface: "eth1".into(),
        };
        let data = SurrealData {
            bridges: vec![DbBridge {
                id: Some(RecordId::new("bridge", "b1")),
                index: 0,
                bridge_name: "br0".into(),
                network_name: "net".into(),
                lab: RecordId::new("lab", "l1"),
                members: vec![member.clone(), member],
            }],
            ..SurrealData::default()
        };
        let summary = ImportSummary::of(&data);
        assert_eq!(summary.bridges, 1);
        assert_eq!(summary.bridge_members, 2);
    }

    #[test]
    fn record_key_refuses_a_missing_id() {
        assert!(record_key(None, USER_TABLE).is_err());
        assert_eq!(
            record_key(Some(&RecordId::new("user", "alice")), USER_TABLE).unwrap(),
            "alice"
        );
    }
}
