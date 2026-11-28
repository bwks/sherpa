//! The records sandboxes have of their own, in the `vm_sandbox` table: one row per sandbox node,
//! with the sandbox's own management addresses and MAC, recorded with the node at `up` and
//! deleted with it.

use anyhow::{Context, Result, bail};
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
        ssh_host_key: cells.optional_text("ssh_host_key")?,
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
            sandbox.ssh_host_key.as_deref().into(),
        ]),
    )
    .await
    .with_context(|| format!("Failed to record the sandbox of node {}", sandbox.node))?;
    Ok(())
}

/// Record `key` as the SSH host key the sandbox of node `node` offers on its own address, or
/// clear it with `None`.
///
/// # Errors
///
/// Fails when `node` is not a node record, its sandbox has no record, or the database
/// cannot be written.
#[instrument(skip(db, key), level = "debug")]
pub async fn update_vm_sandbox_host_key(
    db: &Database,
    node: &RecordId,
    key: Option<&str>,
) -> Result<()> {
    let writer = db.writer().await?;
    let changed = execute(
        &writer.conn,
        queries::vm_sandbox::UPDATE_SSH_HOST_KEY
            .bind([key_of(node, NODE_TABLE)?.into(), key.into()]),
    )
    .await
    .with_context(|| format!("Failed to record the sandbox host key of node {node}"))?;
    if changed == 0 {
        bail!("The sandbox of node {node} has no record");
    }
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
                ssh_host_key: None,
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

    #[tokio::test]
    async fn node_host_keys_019_a_sandbox_s_key_is_kept_in_its_sandbox_record_and_can_be_replaced_or_cleared()
     {
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
        let mut nodes = Vec::new();
        for (index, name) in [(1, "srl1"), (2, "srl2"), (3, "srl3")] {
            let node = create_node(&db, name, index, image.id.clone().unwrap(), lab_id.clone())
                .await
                .unwrap();
            nodes.push(node.id.unwrap());
        }
        for (index, node) in nodes.iter().take(2).enumerate() {
            create_vm_sandbox(
                &db,
                &DbVmSandbox {
                    node: node.clone(),
                    ipv4: format!("172.31.1.{}", 20 + index).parse().unwrap(),
                    ipv6: None,
                    mac: format!("52:54:00:00:00:0{index}"),
                    ssh_host_key: None,
                },
            )
            .await
            .unwrap();
        }
        let key_of_sandbox = async |node: &RecordId| {
            find_vm_sandbox(&db, node)
                .await
                .unwrap()
                .unwrap()
                .ssh_host_key
        };

        update_vm_sandbox_host_key(&db, &nodes[0], Some("ssh-ed25519 AAAAold"))
            .await
            .unwrap();
        assert_eq!(
            key_of_sandbox(&nodes[0]).await.as_deref(),
            Some("ssh-ed25519 AAAAold")
        );
        assert_eq!(key_of_sandbox(&nodes[1]).await, None);

        update_vm_sandbox_host_key(&db, &nodes[0], Some("ssh-ed25519 AAAAnew"))
            .await
            .unwrap();
        assert_eq!(
            key_of_sandbox(&nodes[0]).await.as_deref(),
            Some("ssh-ed25519 AAAAnew")
        );

        update_vm_sandbox_host_key(&db, &nodes[0], None)
            .await
            .unwrap();
        assert_eq!(key_of_sandbox(&nodes[0]).await, None);

        assert!(
            update_vm_sandbox_host_key(&db, &nodes[2], Some("ssh-ed25519 AAAA"))
                .await
                .is_err()
        );
    }
}
