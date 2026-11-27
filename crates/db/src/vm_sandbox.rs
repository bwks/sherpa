//! The records sandboxes have of their own, in the `vm_sandbox` table: one row per sandbox node,
//! with the sandbox's own management addresses and MAC, recorded with the node at `up` and
//! deleted with it.

use anyhow::{Context, Result};
use shared::data::{DbVmSandbox, RecordId};
use tracing::instrument;
use turso::{Row, Value};

use crate::Database;
use crate::persistence::{Cells, LAB_TABLE, NODE_TABLE, key_of};
use crate::queries;
use crate::sql::{execute, query_all, query_optional};

/// A row of the `vm_sandbox` table.
fn vm_sandbox_from_row(row: &Row) -> Result<DbVmSandbox> {
    let mut cells = Cells::new(row);
    Ok(DbVmSandbox {
        node: cells.record(NODE_TABLE, "node")?,
        ipv4: cells.ipv4("ipv4")?,
        ipv6: cells.optional_ipv6("ipv6")?,
        mac: cells.text("mac")?,
    })
}

/// Record the sandbox of `sandbox.node`.
///
/// # Errors
///
/// Fails when the node is not a node record, already has a `vm_sandbox` row, or the database
/// cannot be written.
#[instrument(skip(db), fields(node = %sandbox.node), level = "debug")]
pub async fn create_vm_sandbox(db: &Database, sandbox: &DbVmSandbox) -> Result<()> {
    let writer = db.writer().await?;
    execute(
        &writer.conn,
        queries::vm_sandbox::CREATE.bind([
            key_of(&sandbox.node, NODE_TABLE)?.into(),
            sandbox.ipv4.to_string().into(),
            sandbox
                .ipv6
                .map_or(Value::Null, |ipv6| ipv6.to_string().into()),
            sandbox.mac.as_str().into(),
        ]),
    )
    .await
    .with_context(|| format!("Failed to record the sandbox of node {}", sandbox.node))?;
    Ok(())
}

/// The record of the sandbox of node `node`, if it has one.
///
/// # Errors
///
/// Fails when `node` is not a node record or the database cannot be read.
#[instrument(skip(db), level = "debug")]
pub async fn find_vm_sandbox(db: &Database, node: &RecordId) -> Result<Option<DbVmSandbox>> {
    let conn = db.reader().await?;
    query_optional(
        &conn,
        queries::vm_sandbox::FIND_BY_NODE.bind([key_of(node, NODE_TABLE)?.into()]),
        vm_sandbox_from_row,
    )
    .await
    .with_context(|| format!("Failed to read the sandbox of node {node}"))
}

/// The records of the sandboxes of lab `lab`, in node index order.
///
/// # Errors
///
/// Fails when `lab` is not a lab record or the database cannot be read.
#[instrument(skip(db), level = "debug")]
pub async fn list_vm_sandboxes(db: &Database, lab: &RecordId) -> Result<Vec<DbVmSandbox>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        queries::vm_sandbox::LIST_BY_LAB.bind([key_of(lab, LAB_TABLE)?.into()]),
        vm_sandbox_from_row,
    )
    .await
    .with_context(|| format!("Failed to list the sandboxes of lab {lab}"))
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
    async fn sandbox_069_a_vm_sandbox_row_is_kept_with_its_node_and_goes_with_its_lab() {
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
        let mut sandboxes = Vec::new();
        for (index, name) in [(2, "srl2"), (1, "srl1")] {
            let node = create_node(&db, name, index, image.id.clone().unwrap(), lab_id.clone())
                .await
                .unwrap();
            let sandbox = DbVmSandbox {
                node: node.id.unwrap(),
                ipv4: format!("172.31.1.{}", 22 + index).parse().unwrap(),
                ipv6: (index == 1).then(|| "fd00::17".parse().unwrap()),
                mac: format!("52:54:00:00:00:0{index}"),
            };
            create_vm_sandbox(&db, &sandbox).await.unwrap();
            sandboxes.push(sandbox);
        }
        assert!(create_vm_sandbox(&db, &sandboxes[0]).await.is_err());
        assert_eq!(
            find_vm_sandbox(&db, &sandboxes[1].node).await.unwrap(),
            Some(sandboxes[1].clone())
        );
        assert_eq!(
            list_vm_sandboxes(&db, &lab_id).await.unwrap(),
            vec![sandboxes[1].clone(), sandboxes[0].clone()]
        );

        assert!(
            delete_lab_from(&db, "lab-0001", LabPhase::Creating)
                .await
                .unwrap()
        );
        assert_eq!(
            find_vm_sandbox(&db, &sandboxes[0].node).await.unwrap(),
            None
        );
    }
}
