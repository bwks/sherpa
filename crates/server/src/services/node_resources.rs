//! What each node is given: vCPUs, memory and, for VMs, a VNC console port. They are decided
//! when the node is created or redeployed and stored on the node, so nothing reads them back
//! from the hypervisor.
use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, TcpListener};

use anyhow::{Context, Result, bail};
use tracing::instrument;

use shared::data::{DbNode, NodeConfig, NodeCpuPins, NodeKind, NodeModel, NodeResources, RecordId};
use shared::konst::{
    CISCO_IOL_CONTAINER_OVERHEAD_MIB, JUNIPER_CEVOLVED_CONTAINER_OVERHEAD_MIB,
    VNC_PORT_ALLOCATION_ATTEMPTS, VNC_PORT_FIRST, VNC_PORT_LAST,
};
use topology::validate::check_cpu_pin_overlap;
use virt::connect::Connect;

use crate::validate::check_host_resources;

/// The vCPUs and memory a node gets, and the host CPUs its `cpu` and `boot_disk` settings pin.
/// A VM or unikernel gets the manifest's values, else the image's. A container gets the
/// manifest's as Docker limits, and is not limited in what the manifest leaves out; the image's
/// values are not limits. `juniper_cevolved` sizes its EVO VM from the manifest or image values,
/// so its container's memory limit is that and its overhead, with no CPU quota, which would
/// throttle the EVO VM. IOL takes the manifest or image memory as its own, so its container's
/// limit is that and the launcher's overhead. The VNC port is reserved separately.
pub fn planned_resources(node: &topology::NodeExpanded, image: &NodeConfig) -> NodeResources {
    match image.kind {
        NodeKind::VirtualMachine | NodeKind::Unikernel => {
            let pins = NodeCpuPins::of(&node.name, &node.cpu, &node.boot_disk);
            NodeResources {
                cpu_count: Some(node.cpu.count.unwrap_or(image.cpu.count)),
                memory: Some(node.memory.unwrap_or(image.memory)),
                vnc_port: None,
                cpu_pinning: node.cpu.pinning.clone(),
                qemu_thread_cpus: Some(pins.qemu_thread_cpus).filter(|cpus| !cpus.is_empty()),
            }
        }
        NodeKind::Container if image.model == NodeModel::JuniperCevolved => NodeResources {
            // `check_ptx_platform` leaves room for the overhead.
            memory: Some(
                node.memory
                    .unwrap_or(image.memory)
                    .saturating_add(JUNIPER_CEVOLVED_CONTAINER_OVERHEAD_MIB),
            ),
            ..NodeResources::default()
        },
        NodeKind::Container
            if matches!(image.model, NodeModel::CiscoIol | NodeModel::CiscoIoll2) =>
        {
            NodeResources {
                // IOL's own memory, its `-m`, is the manifest or image value; the launcher and
                // the bridge need the overhead on top.
                cpu_count: node.cpu.count,
                memory: Some(
                    node.memory
                        .unwrap_or(image.memory)
                        .saturating_add(CISCO_IOL_CONTAINER_OVERHEAD_MIB),
                ),
                ..NodeResources::default()
            }
        }
        NodeKind::Container => NodeResources {
            cpu_count: node.cpu.count,
            memory: node.memory,
            ..NodeResources::default()
        },
    }
}

/// The resources stored on a node.
pub fn resources_of(node: &DbNode) -> NodeResources {
    NodeResources {
        cpu_count: node.cpu_count,
        memory: node.memory,
        vnc_port: node.vnc_port,
        cpu_pinning: node.cpu_pinning.clone(),
        qemu_thread_cpus: node.qemu_thread_cpus.clone(),
    }
}

/// Pins another operation was admitted with, for a node that may not be stored yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingPins {
    /// The node's domain name, `<node>-<lab id>`.
    pub domain: String,
    pub pins: NodeCpuPins,
}

/// Check the CPU pins and NUMA node of `nodes`, in lab `lab_id`, against the server: the host CPUs
/// and NUMA nodes it names must exist, and its pins must not overlap those of the nodes already
/// on the server, in any lab, or those `pending` in other operations. A stored node that is one
/// of `nodes`, as in a redeploy, is being replaced, so its stored pins are its own.
#[instrument(skip(db, qemu_conn, nodes, pending), level = "debug")]
pub async fn check_cpu_pins_on_server(
    db: &db::Database,
    qemu_conn: &Connect,
    lab_id: &str,
    nodes: &[&topology::NodeExpanded],
    pending: &[PendingPins],
) -> Result<()> {
    let tuned: Vec<(&topology::NodeExpanded, NodeCpuPins)> = nodes
        .iter()
        .map(|node| {
            (
                *node,
                NodeCpuPins::of(&node.name, &node.cpu, &node.boot_disk),
            )
        })
        .filter(|(node, pins)| !pins.is_empty() || node.numa.node.is_some())
        .collect();
    if tuned.is_empty() {
        return Ok(());
    }

    let host = libvirt::host_cpus(qemu_conn)?;
    let mut pins = Vec::new();
    for (node, node_pins) in tuned {
        check_host_resources(node, host.cpus, host.numa_nodes)?;
        pins.push(node_pins);
    }
    let requested: HashSet<String> = nodes
        .iter()
        .map(|node| domain_name(&node.name, lab_id))
        .collect();
    let lab_ids: HashMap<RecordId, String> = db::list_labs(db)
        .await?
        .into_iter()
        .filter_map(|lab| lab.id.map(|id| (id, lab.lab_id)))
        .collect();
    let stored = db::list_pinned_nodes(db).await?.into_iter().map(|node| {
        let domain = lab_ids
            .get(&node.lab)
            .map(|lab_id| domain_name(&node.name, lab_id));
        (domain, node)
    });
    pins.extend(other_nodes_pins(stored, pending, &requested));
    check_cpu_pin_overlap(&pins)
}

/// The pins of the stored nodes and of the nodes `pending` in other operations, leaving out the
/// `requested` nodes, which are being replaced, and stored nodes that are also pending, whose
/// pending pins are the ones they will have. A stored node whose lab is unknown is kept.
fn other_nodes_pins(
    stored: impl IntoIterator<Item = (Option<String>, DbNode)>,
    pending: &[PendingPins],
    requested: &HashSet<String>,
) -> Vec<NodeCpuPins> {
    let pending_domains: HashSet<&str> = pending.iter().map(|p| p.domain.as_str()).collect();
    let mut pins = Vec::new();
    for (domain, node) in stored {
        if let Some(domain) = &domain
            && (requested.contains(domain) || pending_domains.contains(domain.as_str()))
        {
            continue;
        }
        pins.push(NodeCpuPins {
            node_name: format!("{} (already on the server)", node.name),
            vcpu_cpus: node.cpu_pinning.unwrap_or_default(),
            qemu_thread_cpus: node.qemu_thread_cpus.unwrap_or_default(),
        });
    }
    for entry in pending {
        if requested.contains(&entry.domain) {
            continue;
        }
        pins.push(NodeCpuPins {
            node_name: format!(
                "{} (being created by another operation)",
                entry.pins.node_name
            ),
            ..entry.pins.clone()
        });
    }
    pins
}

/// The libvirt domain name of node `node_name` in lab `lab_id`.
pub fn domain_name(node_name: &str, lab_id: &str) -> String {
    format!("{node_name}-{lab_id}")
}

/// VNC ports in the reserved range that no node holds, lowest first.
pub fn free_vnc_ports(taken: &[u16]) -> Vec<u16> {
    let mut free = Vec::new();
    for port in VNC_PORT_FIRST..=VNC_PORT_LAST {
        if taken.binary_search(&port).is_err() {
            free.push(port);
        }
    }
    free
}

/// Store a node's resources. A VM keeps the VNC port it has, or reserves the lowest port no
/// node holds and nothing on the server listens on (QEMU listens on 127.0.0.1). The port's
/// unique index settles a race with another lab, and the reservation is retried.
#[instrument(skip(db, planned), fields(node = %node.name), level = "debug")]
pub async fn reserve_node_resources(
    db: &db::Database,
    node: &DbNode,
    kind: &NodeKind,
    planned: NodeResources,
) -> Result<DbNode> {
    let id: RecordId = node
        .id
        .clone()
        .with_context(|| format!("Node '{}' has no record id", node.name))?;
    if *kind != NodeKind::VirtualMachine || node.vnc_port.is_some() {
        let vnc_port = if *kind == NodeKind::VirtualMachine {
            node.vnc_port
        } else {
            None
        };
        let resources = NodeResources {
            vnc_port,
            ..planned
        };
        return db::update_node_resources(db, id, &resources).await;
    }

    for attempt in 1..=VNC_PORT_ALLOCATION_ATTEMPTS {
        let taken = db::list_vnc_ports(db).await?;
        let mut port = None;
        for candidate in free_vnc_ports(&taken) {
            if host_port_free(candidate) {
                port = Some(candidate);
                break;
            }
        }
        let Some(port) = port else {
            bail!("No free VNC port between {VNC_PORT_FIRST} and {VNC_PORT_LAST}");
        };
        let resources = NodeResources {
            vnc_port: Some(port),
            ..planned.clone()
        };
        match db::update_node_resources(db, id.clone(), &resources).await {
            Ok(updated) => return Ok(updated),
            Err(error) => {
                if !db::list_vnc_ports(db).await?.contains(&port) {
                    return Err(error);
                }
                tracing::warn!(node = %node.name, port, attempt, "Another node took the VNC port; reserving again");
            }
        }
    }
    bail!(
        "Could not reserve a VNC port for node '{}' after {VNC_PORT_ALLOCATION_ATTEMPTS} attempts",
        node.name
    )
}

/// Whether nothing on the server listens on a port.
fn host_port_free(port: u16) -> bool {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::{NodeCpu, NodeState};

    fn node(cpu_count: Option<u8>, memory: Option<u16>) -> topology::NodeExpanded {
        topology::NodeExpanded {
            name: "dev01".to_string(),
            cpu: NodeCpu {
                count: cpu_count,
                ..NodeCpu::default()
            },
            memory,
            ..Default::default()
        }
    }

    #[test]
    fn vms_and_unikernels_get_the_manifest_values_else_the_images() {
        let image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        assert_eq!(
            planned_resources(&node(Some(16), Some(32768)), &image),
            NodeResources {
                cpu_count: Some(16),
                memory: Some(32768),
                ..NodeResources::default()
            }
        );
        let defaults = planned_resources(&node(None, None), &image);
        assert_eq!(defaults.cpu_count, Some(image.cpu.count));
        assert_eq!(defaults.memory, Some(image.memory));

        let unikernel = NodeConfig::get_model(NodeModel::NanosUnikernel);
        assert_eq!(
            planned_resources(&node(Some(2), None), &unikernel).cpu_count,
            Some(2)
        );
    }

    #[test]
    fn a_tuned_vm_gets_its_pinned_cpus() {
        let image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        let mut tuned = node(Some(2), None);
        tuned.cpu.pinning = Some(vec![9, 25]);
        tuned.cpu.emulator_pinning = Some(vec![24, 8]);
        tuned.boot_disk.io_thread_cpus = Some(vec![8]);
        let resources = planned_resources(&tuned, &image);
        assert_eq!(resources.cpu_pinning, Some(vec![9, 25]));
        assert_eq!(resources.qemu_thread_cpus, Some(vec![8, 24]));

        let unpinned = planned_resources(&node(Some(2), None), &image);
        assert_eq!(unpinned.cpu_pinning, None);
        assert_eq!(unpinned.qemu_thread_cpus, None);
    }

    #[test]
    fn containers_get_the_manifest_limits_only() {
        let image = NodeConfig::get_model(NodeModel::MongoDb);
        assert_eq!(
            planned_resources(&node(Some(4), Some(4096)), &image),
            NodeResources {
                cpu_count: Some(4),
                memory: Some(4096),
                ..NodeResources::default()
            }
        );
        // The image's values are not limits: an unset container is not limited.
        assert_eq!(
            planned_resources(&node(None, None), &image),
            NodeResources::default()
        );
    }

    #[test]
    fn cevolved_gets_its_evo_vm_memory_and_overhead_and_no_cpu_quota() {
        let image = NodeConfig::get_model(NodeModel::JuniperCevolved);
        let defaults = planned_resources(&node(None, None), &image);
        assert_eq!(
            defaults.memory,
            Some(image.memory + JUNIPER_CEVOLVED_CONTAINER_OVERHEAD_MIB)
        );
        assert_eq!(defaults.cpu_count, None);
        let sized = planned_resources(&node(Some(8), Some(16384)), &image);
        assert_eq!(sized.memory, Some(16384 + 2048));
        assert_eq!(sized.cpu_count, None);
    }

    #[test]
    fn iol_gets_its_memory_and_overhead_as_its_limit_and_the_manifest_cpus() {
        for model in [NodeModel::CiscoIol, NodeModel::CiscoIoll2] {
            let image = NodeConfig::get_model(model);
            let defaults = planned_resources(&node(None, None), &image);
            assert_eq!(
                defaults.memory,
                Some(image.memory + CISCO_IOL_CONTAINER_OVERHEAD_MIB),
                "{model}"
            );
            assert_eq!(defaults.cpu_count, None, "{model}");
            let sized = planned_resources(&node(Some(2), Some(1024)), &image);
            assert_eq!(sized.memory, Some(1024 + 128), "{model}");
            assert_eq!(sized.cpu_count, Some(2), "{model}");
        }
    }

    fn stored(name: &str, lab_id: Option<&str>, cpus: Vec<u16>) -> (Option<String>, DbNode) {
        let node = DbNode {
            id: None,
            name: name.to_string(),
            image: RecordId::new("node_image", "x"),
            index: 0,
            lab: RecordId::new("lab", "x"),
            mgmt_ipv4: None,
            mgmt_ipv6: None,
            mgmt_mac: None,
            state: NodeState::Running,
            cpu_count: None,
            memory: None,
            vnc_port: None,
            ssh_host_key: None,
            cpu_pinning: Some(cpus),
            qemu_thread_cpus: None,
        };
        (lab_id.map(|lab_id| domain_name(name, lab_id)), node)
    }

    fn pending(domain: &str, node_name: &str, cpus: Vec<u16>) -> PendingPins {
        PendingPins {
            domain: domain.to_string(),
            pins: NodeCpuPins {
                node_name: node_name.to_string(),
                vcpu_cpus: cpus,
                qemu_thread_cpus: Vec::new(),
            },
        }
    }

    fn names(pins: &[NodeCpuPins]) -> Vec<&str> {
        pins.iter().map(|pin| pin.node_name.as_str()).collect()
    }

    #[test]
    fn pins_of_other_nodes_include_stored_and_pending_ones() {
        let pins = other_nodes_pins(
            [stored("a", Some("lab1"), vec![1])],
            &[pending("b-lab2", "b", vec![2])],
            &HashSet::new(),
        );
        assert_eq!(
            names(&pins),
            [
                "a (already on the server)",
                "b (being created by another operation)"
            ]
        );
        assert_eq!(pins[1].vcpu_cpus, vec![2]);
    }

    #[test]
    fn a_stored_node_that_is_also_pending_counts_once_with_its_pending_pins() {
        let pins = other_nodes_pins(
            [stored("a", Some("lab1"), vec![1])],
            &[pending("a-lab1", "a", vec![3])],
            &HashSet::new(),
        );
        assert_eq!(names(&pins), ["a (being created by another operation)"]);
        assert_eq!(pins[0].vcpu_cpus, vec![3]);
    }

    #[test]
    fn a_requested_node_does_not_overlap_its_own_stored_or_pending_pins() {
        let requested = HashSet::from(["a-lab1".to_string()]);
        let pins = other_nodes_pins(
            [stored("a", Some("lab1"), vec![1])],
            &[pending("a-lab1", "a", vec![1])],
            &requested,
        );
        assert_eq!(pins, []);
    }

    #[test]
    fn a_stored_node_whose_lab_is_unknown_is_kept() {
        let requested = HashSet::from(["a-lab1".to_string()]);
        let pins = other_nodes_pins([stored("a", None, vec![1])], &[], &requested);
        assert_eq!(names(&pins), ["a (already on the server)"]);
    }

    #[test]
    fn free_ports_skip_taken_ones_and_stay_in_the_range() {
        let free = free_vnc_ports(&[5900, 5902]);
        assert_eq!(&free[..3], &[5901, 5903, 5904]);
        assert_eq!(free.last(), Some(&VNC_PORT_LAST));
        assert_eq!(
            free.len(),
            usize::from(VNC_PORT_LAST - VNC_PORT_FIRST + 1) - 2
        );
    }
}
