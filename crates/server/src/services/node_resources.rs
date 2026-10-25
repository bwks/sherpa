//! What each node is given: vCPUs, memory and, for VMs, a VNC console port. They are decided
//! when the node is created or redeployed and stored on the node, so nothing reads them back
//! from the hypervisor.
use std::net::{Ipv4Addr, TcpListener};

use anyhow::{Context, Result, bail};
use tracing::instrument;

use shared::data::{DbNode, NodeConfig, NodeKind, NodeResources, RecordId};
use shared::konst::{VNC_PORT_ALLOCATION_ATTEMPTS, VNC_PORT_FIRST, VNC_PORT_LAST};

/// The vCPUs and memory a node gets: the manifest's values, else the image's. Containers get
/// none, as Sherpa does not limit them. The VNC port is reserved separately.
pub fn planned_resources(node: &topology::NodeExpanded, image: &NodeConfig) -> NodeResources {
    match image.kind {
        NodeKind::VirtualMachine | NodeKind::Unikernel => NodeResources {
            cpu_count: Some(node.cpu_count.unwrap_or(image.cpu_count)),
            memory: Some(node.memory.unwrap_or(image.memory)),
            vnc_port: None,
        },
        NodeKind::Container => NodeResources::default(),
    }
}

/// The resources stored on a node.
pub fn resources_of(node: &DbNode) -> NodeResources {
    NodeResources {
        cpu_count: node.cpu_count,
        memory: node.memory,
        vnc_port: node.vnc_port,
    }
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
    use shared::data::NodeModel;

    fn node(cpu_count: Option<u8>, memory: Option<u16>) -> topology::NodeExpanded {
        topology::NodeExpanded {
            name: "dev01".to_string(),
            cpu_count,
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
                vnc_port: None,
            }
        );
        let defaults = planned_resources(&node(None, None), &image);
        assert_eq!(defaults.cpu_count, Some(image.cpu_count));
        assert_eq!(defaults.memory, Some(image.memory));

        let unikernel = NodeConfig::get_model(NodeModel::NanosUnikernel);
        assert_eq!(
            planned_resources(&node(Some(2), None), &unikernel).cpu_count,
            Some(2)
        );
    }

    #[test]
    fn containers_get_no_limits() {
        let image = NodeConfig::get_model(NodeModel::MongoDb);
        assert_eq!(
            planned_resources(&node(Some(4), Some(4096)), &image),
            NodeResources::default()
        );
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
