use std::error::Error;
use std::fmt;

use anyhow::{Context, Result};
use shared::data::{NodeCpuPins, RecordId};
use shared::error::CpuPinRole;
use tracing::instrument;
use turso::{Connection, Row, Value};

use super::read::pin_from_row;
use crate::Database;
use crate::persistence::{Cells, LAB_TABLE, enum_text, key_of};
use crate::queries;
use crate::sql::{execute, query_all, query_optional};

/// The message of the `cpu_pin_held` trigger in migration `0023_cpu_pin.sql`.
const HELD_BY_ANOTHER_NODE: &str = "Host CPU is already pinned by another node";

/// The database refused to pin a host CPU to a node: another node holds it, and one of the two
/// pins it to its vCPUs. The trigger names no node, so neither does this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuPinTaken {
    /// The node the CPU was claimed for, by its name in its lab.
    pub node_name: String,
    pub host_cpu: u16,
    /// What the node would pin to the CPU.
    pub requested_role: CpuPinRole,
    /// What the node holding the CPU pins to it.
    pub held_role: CpuPinRole,
}

impl fmt::Display for CpuPinTaken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Host CPU {}, which node '{}' would pin to its {}, is pinned to the {} of another \
             node. Try again once that node is destroyed or redeployed without it",
            self.host_cpu,
            self.node_name,
            self.requested_role.describe(),
            self.held_role.describe(),
        )
    }
}

impl Error for CpuPinTaken {}

/// Claim the host CPUs `nodes` of lab `lab` pin, in one transaction: a row for each CPU and
/// role a node does not hold yet. The rows a node already holds are kept, so a redeploy holds
/// its old and its new pins together until [`retain_cpu_pins`] keeps one set.
///
/// # Errors
///
/// Fails with [`CpuPinTaken`] when the table's trigger refuses a CPU another node holds against
/// the pinning rules, and then nothing is claimed. Fails too when `lab` is not a lab record or
/// the database cannot be written.
#[instrument(skip(db, nodes), fields(%lab), level = "debug")]
pub async fn claim_cpu_pins(db: &Database, lab: &RecordId, nodes: &[NodeCpuPins]) -> Result<()> {
    let lab_key = key_of(lab, LAB_TABLE)?;
    let mut writer = db.writer().await?;
    let tx = writer
        .conn
        .transaction()
        .await
        .context("Failed to start the CPU pin transaction")?;
    for node in nodes {
        for (host_cpu, role) in pin_rows(node) {
            let params = pin_values(&lab_key, &node.node_name, host_cpu, role)?;
            let Err(error) = execute(&tx, queries::cpu_pin::CLAIM.bind(params)).await else {
                continue;
            };
            if !error
                .chain()
                .any(|cause| cause.to_string().contains(HELD_BY_ANOTHER_NODE))
            {
                return Err(error);
            }
            let held_role = holder_role(&tx, &lab_key, &node.node_name, host_cpu, role)
                .await
                .context(error)?;
            return Err(CpuPinTaken {
                node_name: node.node_name.clone(),
                host_cpu,
                requested_role: role,
                held_role,
            }
            .into());
        }
    }
    tx.commit().await.context("Failed to commit the CPU pins")?;
    Ok(())
}

/// Keep only `pins` of node `pins.node_name` in lab `lab`: delete the node's other rows, in one
/// transaction. A redeploy keeps its new pins once the old domain is gone, or its old ones when
/// it fails before then. A pin in `pins` the node does not hold is not claimed.
///
/// # Errors
///
/// Fails when `lab` is not a lab record or the database cannot be written.
#[instrument(skip(db, pins), fields(%lab, node_name = %pins.node_name), level = "debug")]
pub async fn retain_cpu_pins(db: &Database, lab: &RecordId, pins: &NodeCpuPins) -> Result<()> {
    let lab_key = key_of(lab, LAB_TABLE)?;
    let mut writer = db.writer().await?;
    let tx = writer
        .conn
        .transaction()
        .await
        .context("Failed to start the CPU pin transaction")?;
    let held = query_all(
        &tx,
        queries::cpu_pin::LIST_BY_NODE
            .bind([lab_key.as_str().into(), pins.node_name.as_str().into()]),
        pin_from_row,
    )
    .await?;
    let kept = pin_rows(pins);
    for row in held
        .into_iter()
        .filter(|row| !kept.contains(&(row.host_cpu, row.role)))
    {
        execute(
            &tx,
            queries::cpu_pin::DELETE.bind(pin_values(
                &lab_key,
                &pins.node_name,
                row.host_cpu,
                row.role,
            )?),
        )
        .await?;
    }
    tx.commit().await.context("Failed to commit the CPU pins")?;
    Ok(())
}

/// Each host CPU and role of `pins`, one per pair.
fn pin_rows(pins: &NodeCpuPins) -> Vec<(u16, CpuPinRole)> {
    let mut rows: Vec<(u16, CpuPinRole)> = pins
        .vcpu_cpus
        .iter()
        .map(|&cpu| (cpu, CpuPinRole::Vcpu))
        .chain(
            pins.qemu_thread_cpus
                .iter()
                .map(|&cpu| (cpu, CpuPinRole::Thread)),
        )
        .collect();
    rows.sort_unstable();
    rows.dedup();
    rows
}

/// A row's key columns as query parameters.
fn pin_values(
    lab_key: &str,
    node_name: &str,
    host_cpu: u16,
    role: CpuPinRole,
) -> Result<[Value; 4]> {
    Ok([
        lab_key.into(),
        node_name.into(),
        host_cpu.into(),
        enum_text(&role, "role")?.into(),
    ])
}

fn role_from_row(row: &Row) -> Result<CpuPinRole> {
    Cells::new(row).enumeration("role")
}

/// What another node pins to `host_cpu` against the rules beside `role` for node `node_name`:
/// its vCPUs when it does, else its threads.
async fn holder_role(
    conn: &Connection,
    lab_key: &str,
    node_name: &str,
    host_cpu: u16,
    role: CpuPinRole,
) -> Result<CpuPinRole> {
    query_optional(
        conn,
        queries::cpu_pin::HOLDER_ROLE.bind(pin_values(lab_key, node_name, host_cpu, role)?),
        role_from_row,
    )
    .await?
    .context("The host CPU is no longer held by another node")
}

#[cfg(test)]
mod tests {
    use shared::data::{LabPhase, ModelKind, NodeModel, NodeState};

    use super::*;
    use crate::{
        DefaultRequest, LabDetails, LabNetworks, create_lab, create_node, create_user,
        delete_lab_from, list_cpu_pins, open_in_memory, update_node_state_from, upsert_node_image,
    };

    fn pins(node_name: &str, vcpus: &[u16], threads: &[u16]) -> NodeCpuPins {
        NodeCpuPins {
            node_name: node_name.to_string(),
            vcpu_cpus: vcpus.to_vec(),
            qemu_thread_cpus: threads.to_vec(),
        }
    }

    fn networks(octet: u8) -> LabNetworks {
        LabNetworks {
            management_network: format!("172.31.{octet}.0/24"),
            gateway_ipv4: format!("172.31.{octet}.1"),
            router_ipv4: format!("172.31.{octet}.2"),
            management_network_v6: None,
            loopback_network_v6: None,
            gateway_ipv6: None,
            router_ipv6: None,
        }
    }

    /// A database with labs `lab-0001` and `lab-0002`, both owned by alice. Returns their
    /// records.
    async fn two_labs() -> (Database, RecordId, RecordId) {
        let db = open_in_memory().await.unwrap();
        crate::apply_schema(&db).await.unwrap();
        let user = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![])
            .await
            .unwrap();
        let mut labs = Vec::new();
        for octet in 1..=2 {
            let lab = create_lab(
                &db,
                &format!("Lab {octet}"),
                &format!("lab-000{octet}"),
                &user,
                &networks(octet),
                &LabDetails::default(),
            )
            .await
            .unwrap();
            labs.push(lab.id.unwrap());
        }
        let second = labs.pop().unwrap();
        let first = labs.pop().unwrap();
        (db, first, second)
    }

    /// The pins `node_name` of `lab` holds.
    async fn held(db: &Database, lab: &RecordId, node_name: &str) -> Option<NodeCpuPins> {
        list_cpu_pins(db)
            .await
            .unwrap()
            .into_iter()
            .find(|node| node.lab == *lab && node.pins.node_name == node_name)
            .map(|node| node.pins)
    }

    fn taken(result: Result<()>) -> CpuPinTaken {
        let error = result.unwrap_err();
        error
            .downcast_ref::<CpuPinTaken>()
            .unwrap_or_else(|| panic!("not a CpuPinTaken: {error:#}"))
            .clone()
    }

    #[tokio::test]
    async fn a_vcpu_cpu_another_node_holds_is_refused_and_nothing_is_claimed() {
        let (db, first, second) = two_labs().await;
        claim_cpu_pins(&db, &first, &[pins("n1", &[2, 3], &[])])
            .await
            .unwrap();
        // Across labs, and in the same lab under another name.
        for lab in [&second, &first] {
            let refused = taken(claim_cpu_pins(&db, lab, &[pins("n2", &[5, 3], &[])]).await);
            assert_eq!(
                refused,
                CpuPinTaken {
                    node_name: "n2".to_string(),
                    host_cpu: 3,
                    requested_role: CpuPinRole::Vcpu,
                    held_role: CpuPinRole::Vcpu,
                }
            );
            assert_eq!(held(&db, lab, "n2").await, None, "CPU 5 was claimed");
        }
        // A refusal in the last node of a request leaves none of the earlier nodes' rows.
        let refused = claim_cpu_pins(
            &db,
            &second,
            &[pins("n3", &[6], &[]), pins("n4", &[2], &[])],
        )
        .await;
        assert_eq!(taken(refused).node_name, "n4");
        assert_eq!(held(&db, &second, "n3").await, None);
    }

    #[tokio::test]
    async fn qemu_threads_share_cpus_but_never_with_a_vcpu() {
        let (db, first, second) = two_labs().await;
        claim_cpu_pins(&db, &first, &[pins("n1", &[2], &[8])])
            .await
            .unwrap();

        let vcpu_on_thread = taken(claim_cpu_pins(&db, &second, &[pins("n2", &[8], &[])]).await);
        assert_eq!(
            (vcpu_on_thread.requested_role, vcpu_on_thread.held_role),
            (CpuPinRole::Vcpu, CpuPinRole::Thread)
        );
        let thread_on_vcpu = taken(claim_cpu_pins(&db, &second, &[pins("n2", &[], &[2])]).await);
        assert_eq!(
            (thread_on_vcpu.requested_role, thread_on_vcpu.held_role),
            (CpuPinRole::Thread, CpuPinRole::Vcpu)
        );

        claim_cpu_pins(&db, &second, &[pins("n2", &[4], &[8])])
            .await
            .unwrap();
        assert_eq!(held(&db, &second, "n2").await, Some(pins("n2", &[4], &[8])));
    }

    #[tokio::test]
    async fn a_redeploying_node_overlaps_only_itself_and_keeps_one_set_after() {
        let (db, first, second) = two_labs().await;
        claim_cpu_pins(&db, &first, &[pins("n1", &[2, 3], &[])])
            .await
            .unwrap();
        // The redeploy claims its new pins beside the old ones, a vCPU CPU moving to a thread.
        claim_cpu_pins(&db, &first, &[pins("n1", &[3, 4], &[2])])
            .await
            .unwrap();
        assert_eq!(
            held(&db, &first, "n1").await,
            Some(pins("n1", &[2, 3, 4], &[2]))
        );
        // While it runs, no other node may take the old or the new ones.
        for cpu in [2, 4] {
            taken(claim_cpu_pins(&db, &second, &[pins("n2", &[cpu], &[])]).await);
        }

        // Once the old domain is gone, the node keeps its new pins.
        retain_cpu_pins(&db, &first, &pins("n1", &[3, 4], &[2]))
            .await
            .unwrap();
        assert_eq!(
            held(&db, &first, "n1").await,
            Some(pins("n1", &[3, 4], &[2]))
        );
        taken(claim_cpu_pins(&db, &second, &[pins("n2", &[2], &[])]).await);
        claim_cpu_pins(&db, &second, &[pins("n2", &[], &[2])])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_redeploy_that_fails_before_teardown_keeps_its_old_pins() {
        let (db, first, second) = two_labs().await;
        claim_cpu_pins(&db, &first, &[pins("n1", &[2], &[])])
            .await
            .unwrap();
        claim_cpu_pins(&db, &first, &[pins("n1", &[5], &[])])
            .await
            .unwrap();
        retain_cpu_pins(&db, &first, &pins("n1", &[2], &[]))
            .await
            .unwrap();
        assert_eq!(held(&db, &first, "n1").await, Some(pins("n1", &[2], &[])));
        claim_cpu_pins(&db, &second, &[pins("n2", &[5], &[])])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn stopping_keeps_the_pins_and_deleting_the_node_or_lab_gives_them_back() {
        let (db, first, second) = two_labs().await;
        let image = upsert_node_image(
            &db,
            ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
            DefaultRequest::Make,
        )
        .await
        .unwrap();
        let node = create_node(&db, "n1", 1, image.id.clone().unwrap(), first.clone())
            .await
            .unwrap();
        claim_cpu_pins(&db, &first, &[pins("n1", &[2], &[8])])
            .await
            .unwrap();
        claim_cpu_pins(&db, &first, &[pins("n9", &[9], &[])])
            .await
            .unwrap();

        update_node_state_from(
            &db,
            node.id.clone().unwrap(),
            NodeState::Building,
            NodeState::Stopped,
        )
        .await
        .unwrap();
        assert_eq!(held(&db, &first, "n1").await, Some(pins("n1", &[2], &[8])));

        {
            let writer = db.writer().await.unwrap();
            writer
                .conn
                .execute(
                    "DELETE FROM node WHERE id = ?1",
                    [key_of(node.id.as_ref().unwrap(), "node").unwrap()],
                )
                .await
                .unwrap();
        }
        assert_eq!(held(&db, &first, "n1").await, None);
        assert!(held(&db, &first, "n9").await.is_some());

        assert!(
            delete_lab_from(&db, "lab-0001", LabPhase::Creating)
                .await
                .unwrap()
        );
        assert_eq!(list_cpu_pins(&db).await.unwrap(), Vec::new());
        claim_cpu_pins(&db, &second, &[pins("n2", &[2, 9], &[])])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn the_trigger_refuses_a_conflicting_row_written_by_hand() {
        let (db, first, second) = two_labs().await;
        claim_cpu_pins(&db, &first, &[pins("n1", &[2], &[8])])
            .await
            .unwrap();
        let first_key = key_of(&first, LAB_TABLE).unwrap();
        let second_key = key_of(&second, LAB_TABLE).unwrap();
        let writer = db.writer().await.unwrap();
        for (lab, node_name, cpu, role) in [
            (&second_key, "n2", 2, "vcpu"),
            (&second_key, "n2", 2, "thread"),
            (&second_key, "n2", 8, "vcpu"),
            (&first_key, "n2", 2, "vcpu"),
        ] {
            let error = writer
                .conn
                .execute(
                    "INSERT INTO cpu_pin (lab, node_name, host_cpu, role) \
                     VALUES (?1, ?2, ?3, ?4)",
                    (lab.as_str(), node_name, cpu, role),
                )
                .await
                .unwrap_err();
            assert!(
                error.to_string().contains(HELD_BY_ANOTHER_NODE),
                "{lab} {node_name} {cpu} {role}: {error}"
            );
        }
        writer
            .conn
            .execute(
                "INSERT INTO cpu_pin (lab, node_name, host_cpu, role) VALUES (?1, 'n2', 8, 'thread')",
                [second_key.as_str()],
            )
            .await
            .unwrap();
        let error = writer
            .conn
            .execute("UPDATE cpu_pin SET host_cpu = 3 WHERE node_name = 'n2'", ())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("never changed"), "{error}");
    }
}
