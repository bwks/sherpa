//! The management addresses sandboxes have of their own, in the `sandbox_address` table: one row
//! per sandbox node, recorded with the node at `up` and deleted with it.

use anyhow::{Context, Result};
use shared::data::{DbSandboxAddress, RecordId};
use tracing::instrument;
use turso::{Row, Value};

use crate::Database;
use crate::persistence::{Cells, LAB_TABLE, NODE_TABLE, key_of};
use crate::queries;
use crate::sql::{execute, query_all, query_optional};

/// A row of the `sandbox_address` table.
fn sandbox_address_from_row(row: &Row) -> Result<DbSandboxAddress> {
    let mut cells = Cells::new(row);
    Ok(DbSandboxAddress {
        node: cells.record(NODE_TABLE, "node")?,
        ipv4: cells.ipv4("ipv4")?,
        ipv6: cells.optional_ipv6("ipv6")?,
        mac: cells.text("mac")?,
    })
}

/// Record the management address of the sandbox of `address.node`.
///
/// # Errors
///
/// Fails when the node is not a node record, already has a sandbox address, or the database
/// cannot be written.
#[instrument(skip(db), fields(node = %address.node), level = "debug")]
pub async fn create_sandbox_address(db: &Database, address: &DbSandboxAddress) -> Result<()> {
    let writer = db.writer().await?;
    execute(
        &writer.conn,
        queries::sandbox_address::CREATE.bind([
            key_of(&address.node, NODE_TABLE)?.into(),
            address.ipv4.to_string().into(),
            address
                .ipv6
                .map_or(Value::Null, |ipv6| ipv6.to_string().into()),
            address.mac.as_str().into(),
        ]),
    )
    .await
    .with_context(|| {
        format!(
            "Failed to record the sandbox address of node {}",
            address.node
        )
    })?;
    Ok(())
}

/// The management address of the sandbox of node `node`, if it has one.
///
/// # Errors
///
/// Fails when `node` is not a node record or the database cannot be read.
#[instrument(skip(db), level = "debug")]
pub async fn find_sandbox_address(
    db: &Database,
    node: &RecordId,
) -> Result<Option<DbSandboxAddress>> {
    let conn = db.reader().await?;
    query_optional(
        &conn,
        queries::sandbox_address::FIND_BY_NODE.bind([key_of(node, NODE_TABLE)?.into()]),
        sandbox_address_from_row,
    )
    .await
    .with_context(|| format!("Failed to read the sandbox address of node {node}"))
}

/// The management addresses of the sandboxes of lab `lab`, in node index order.
///
/// # Errors
///
/// Fails when `lab` is not a lab record or the database cannot be read.
#[instrument(skip(db), level = "debug")]
pub async fn list_sandbox_addresses(
    db: &Database,
    lab: &RecordId,
) -> Result<Vec<DbSandboxAddress>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        queries::sandbox_address::LIST_BY_LAB.bind([key_of(lab, LAB_TABLE)?.into()]),
        sandbox_address_from_row,
    )
    .await
    .with_context(|| format!("Failed to list the sandbox addresses of lab {lab}"))
}

#[cfg(test)]
mod tests {
    use shared::data::{LabPhase, ModelKind, NodeModel};

    use super::*;
    use crate::{
        DefaultRequest, LabDetails, LabNetworks, apply_schema, create_lab, create_node,
        create_user, delete_lab_from, open_in_memory, upsert_node_image,
    };

    #[tokio::test]
    async fn sandbox_069_a_sandbox_address_is_kept_with_its_node_and_goes_with_its_lab() {
        let db = open_in_memory().await.unwrap();
        apply_schema(&db).await.unwrap();
        let user = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![])
            .await
            .unwrap();
        let lab = create_lab(
            &db,
            "Lab",
            "lab-0001",
            &user,
            &LabNetworks {
                management_network: "172.31.1.0/24".to_owned(),
                gateway_ipv4: "172.31.1.1".to_owned(),
                router_ipv4: "172.31.1.2".to_owned(),
                management_network_v6: None,
                loopback_network_v6: None,
                gateway_ipv6: None,
                router_ipv6: None,
            },
            &LabDetails::default(),
        )
        .await
        .unwrap();
        let lab_id = lab.id.unwrap();
        let image = upsert_node_image(
            &db,
            ModelKind::new(NodeModel::NokiaSrlinux, None)
                .unwrap()
                .node_config(),
            DefaultRequest::Make,
        )
        .await
        .unwrap();
        let mut addresses = Vec::new();
        for (index, name) in [(2, "srl2"), (1, "srl1")] {
            let node = create_node(&db, name, index, image.id.clone().unwrap(), lab_id.clone())
                .await
                .unwrap();
            let address = DbSandboxAddress {
                node: node.id.unwrap(),
                ipv4: format!("172.31.1.{}", 22 + index).parse().unwrap(),
                ipv6: (index == 1).then(|| "fd00::17".parse().unwrap()),
                mac: format!("52:54:00:00:00:0{index}"),
            };
            create_sandbox_address(&db, &address).await.unwrap();
            addresses.push(address);
        }
        assert!(create_sandbox_address(&db, &addresses[0]).await.is_err());
        assert_eq!(
            find_sandbox_address(&db, &addresses[1].node).await.unwrap(),
            Some(addresses[1].clone())
        );
        assert_eq!(
            list_sandbox_addresses(&db, &lab_id).await.unwrap(),
            vec![addresses[1].clone(), addresses[0].clone()]
        );

        assert!(
            delete_lab_from(&db, "lab-0001", LabPhase::Creating)
                .await
                .unwrap()
        );
        assert_eq!(
            find_sandbox_address(&db, &addresses[0].node).await.unwrap(),
            None
        );
    }
}
