use anyhow::{Context, Result};
use shared::data::{DbCpuPins, NodeCpuPins, RecordId};
use shared::error::CpuPinRole;
use tracing::instrument;
use turso::Row;

use crate::Database;
use crate::persistence::{Cells, LAB_TABLE};
use crate::queries;
use crate::sql::query_all;

/// One row of the `cpu_pin` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PinRow {
    pub(super) lab: RecordId,
    pub(super) node_name: String,
    pub(super) host_cpu: u16,
    pub(super) role: CpuPinRole,
}

pub(super) fn pin_from_row(row: &Row) -> Result<PinRow> {
    let mut cells = Cells::new(row);
    Ok(PinRow {
        lab: cells.record(LAB_TABLE, "lab")?,
        node_name: cells.text("node_name")?,
        host_cpu: cells.integer("host_cpu")?,
        role: cells.enumeration("role")?,
    })
}

/// Every node's pinned host CPUs, in every lab: one entry per node that holds any.
///
/// # Errors
///
/// Fails when the database cannot be read.
#[instrument(skip(db), level = "debug")]
pub async fn list_cpu_pins(db: &Database) -> Result<Vec<DbCpuPins>> {
    let conn = db.reader().await?;
    let rows = query_all(&conn, queries::cpu_pin::LIST.bind([]), pin_from_row)
        .await
        .context("Failed to list the pinned host CPUs")?;
    Ok(group_by_node(rows))
}

/// The rows, ordered by lab and node, as one entry per node with each list ascending.
fn group_by_node(rows: Vec<PinRow>) -> Vec<DbCpuPins> {
    let mut nodes: Vec<DbCpuPins> = Vec::new();
    for row in rows {
        let same_node = nodes
            .last()
            .is_some_and(|last| last.lab == row.lab && last.pins.node_name == row.node_name);
        if !same_node {
            nodes.push(DbCpuPins {
                lab: row.lab.clone(),
                pins: NodeCpuPins {
                    node_name: row.node_name.clone(),
                    vcpu_cpus: Vec::new(),
                    qemu_thread_cpus: Vec::new(),
                },
            });
        }
        if let Some(node) = nodes.last_mut() {
            match row.role {
                CpuPinRole::Vcpu => node.pins.vcpu_cpus.push(row.host_cpu),
                CpuPinRole::Thread => node.pins.qemu_thread_cpus.push(row.host_cpu),
            }
        }
    }
    for node in &mut nodes {
        node.pins.vcpu_cpus.sort_unstable();
        node.pins.qemu_thread_cpus.sort_unstable();
    }
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(lab: &str, node_name: &str, host_cpu: u16, role: CpuPinRole) -> PinRow {
        PinRow {
            lab: RecordId::new(LAB_TABLE, lab),
            node_name: node_name.to_string(),
            host_cpu,
            role,
        }
    }

    #[test]
    fn rows_group_into_one_entry_per_node_with_sorted_lists() {
        let grouped = group_by_node(vec![
            row("a", "n1", 8, CpuPinRole::Thread),
            row("a", "n1", 3, CpuPinRole::Vcpu),
            row("a", "n1", 2, CpuPinRole::Vcpu),
            row("a", "n2", 4, CpuPinRole::Vcpu),
            row("b", "n1", 5, CpuPinRole::Thread),
        ]);
        assert_eq!(grouped.len(), 3);
        assert_eq!(grouped[0].pins.vcpu_cpus, vec![2, 3]);
        assert_eq!(grouped[0].pins.qemu_thread_cpus, vec![8]);
        assert_eq!(grouped[1].pins.node_name, "n2");
        assert_eq!(grouped[2].lab, RecordId::new(LAB_TABLE, "b"));
        assert_eq!(grouped[2].pins.vcpu_cpus, Vec::<u16>::new());
        assert_eq!(grouped[2].pins.qemu_thread_cpus, vec![5]);
    }
}
