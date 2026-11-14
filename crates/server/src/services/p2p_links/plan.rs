//! How a node's link ends are made again, planned from the database and the lab's saved
//! manifest: what a cold boot by resume and the scanner's repair hand to re-attach.

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use tracing::instrument;

use super::reattach::{LabLinks, RecreatedEnd};
use super::veths::{
    ContainerNode, container_veths, data_interface_indices, node_bridge_interfaces, node_links,
};

use shared::data::{DbBridge, DbLink, DbNode, NodeConfig, NodeKind, RecordId};
use shared::konst::SHERPA_LABS_PATH;
use shared::util::load_saved_manifest;
use shared::validators::effective_data_interface_count;
use topology::Manifest;

/// A node whose link ends are planned.
pub(crate) struct PlannedNode<'a> {
    pub(crate) lab_id: &'a str,
    pub(crate) node: &'a DbNode,
    pub(crate) node_id: &'a RecordId,
    pub(crate) kind: &'a NodeKind,
    pub(crate) image: Option<&'a NodeConfig>,
}

/// Load the manifest saved by `up`. Labs without a readable manifest use the defaults: the
/// model's interface count, and the default readiness settings.
#[instrument(level = "debug")]
pub(crate) fn load_lab_manifest(lab_id: &str) -> Option<Manifest> {
    let lab_dir = PathBuf::from(format!("{SHERPA_LABS_PATH}/{lab_id}"));
    let value = match load_saved_manifest(&lab_dir) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(lab_id = %lab_id, error = ?error, "No saved manifest; using the defaults");
            return None;
        }
    };
    match serde_json::from_value(value) {
        Ok(manifest) => Some(manifest),
        Err(error) => {
            tracing::warn!(lab_id = %lab_id, error = ?error, "Saved manifest unreadable; using the defaults");
            None
        }
    }
}

/// How the node's link ends are made again, or `None` when it has no ends to re-attach.
///
/// libvirt makes a VM's or a unikernel's taps when it boots the domain. A container started
/// from exited, or restarted, has a new network namespace with none of its veths, so each of
/// its data interfaces is planned again, as redeploy does, with the saved manifest's interface
/// count. A container on no link has Docker networks, which Docker connects again.
pub(crate) fn recreated_end(
    node: &PlannedNode<'_>,
    manifest: Option<&Manifest>,
    lab_links: &LabLinks,
    bridges: Result<&[DbBridge], &String>,
) -> Option<RecreatedEnd> {
    match node.kind {
        NodeKind::VirtualMachine | NodeKind::Unikernel => Some(RecreatedEnd::Domain),
        NodeKind::Container => {
            let links = node_links(lab_links.links(), node.node_id);
            if links.is_empty() {
                return None;
            }
            Some(
                container_end(node, manifest, &links, bridges)
                    .unwrap_or_else(|error| RecreatedEnd::Unplanned(format!("{error:#}"))),
            )
        }
    }
}

/// The container's veths, planned from its links and bridges in the database and its data
/// interface count.
fn container_end(
    node: &PlannedNode<'_>,
    manifest: Option<&Manifest>,
    links: &[&DbLink],
    bridges: Result<&[DbBridge], &String>,
) -> Result<RecreatedEnd> {
    let name = node.node.name.as_str();
    let image = node
        .image
        .ok_or_else(|| anyhow!("Node image of {name} not found"))?;
    let bridges = bridges.map_err(|error| anyhow!("Failed to list the lab's bridges: {error}"))?;
    let manifest_count = manifest
        .and_then(|manifest| manifest.nodes.iter().find(|n| n.name == name))
        .and_then(|n| n.interfaces.data_count);
    let count = effective_data_interface_count(name, manifest_count, image)?;
    let veths = container_veths(
        &ContainerNode {
            lab_id: node.lab_id,
            name,
            index: node.node.index,
            model: image.model,
            record: node.node_id,
        },
        links,
        &node_bridge_interfaces(bridges, node.node_id),
        data_interface_indices(image, count, name)?,
    )?;
    Ok(RecreatedEnd::Container {
        container: format!("{name}-{}", node.lab_id),
        veths,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use shared::data::{BridgeKind, LinkImpairment, NodeModel, NodeState};

    fn manifest(text: &str) -> Manifest {
        toml::from_str(text).unwrap()
    }

    fn db_node(name: &str) -> DbNode {
        DbNode {
            id: None,
            name: name.to_string(),
            image: RecordId::new("node_image", "x"),
            index: 3,
            lab: RecordId::new("lab", "x"),
            mgmt_ipv4: None,
            mgmt_ipv6: None,
            mgmt_mac: None,
            state: NodeState::Running,
            cpu_count: None,
            memory: None,
            vnc_port: None,
            ssh_host_key: None,
            cpu_pinning: None,
            qemu_thread_cpus: None,
        }
    }

    fn link(index: u16, a: &str, b: &str) -> DbLink {
        DbLink {
            id: None,
            index,
            kind: BridgeKind::P2p,
            node_a: RecordId::new("node", a),
            node_b: RecordId::new("node", b),
            int_a: "eth1".to_string(),
            int_b: "eth1".to_string(),
            lab: RecordId::new("lab", "x"),
            tap_a: format!("tpa{index}-lab1"),
            tap_b: format!("tpb{index}-lab1"),
            impairment_a_to_b: LinkImpairment::default(),
            impairment_b_to_a: LinkImpairment::default(),
        }
    }

    fn linked_lab() -> LabLinks {
        LabLinks::new(
            "lab1",
            vec![link(0, "vm1", "uk1"), link(1, "vm1", "ctr1")],
            &[],
            HashMap::new(),
        )
    }

    fn end_of(name: &str, kind: &NodeKind, image: Option<&NodeConfig>) -> Option<RecreatedEnd> {
        let node = db_node(name);
        let node_id = RecordId::new("node", name);
        let bridges: Vec<DbBridge> = vec![];
        recreated_end(
            &PlannedNode {
                lab_id: "lab1",
                node: &node,
                node_id: &node_id,
                kind,
                image,
            },
            Some(&manifest(
                "name = \"l\"\n[[nodes]]\nname = \"ctr1\"\nmodel = \"frr_linux\"\n[nodes.interfaces]\ndata_count = 2\n",
            )),
            &linked_lab(),
            Ok(&bridges),
        )
    }

    #[test]
    fn a_cold_booted_unikernel_has_its_links_reattached() {
        // M8.1: only cold-booted VMs used to be re-attached; a unikernel's new tap was left
        // without a redirect, and its peer's pointed at the old one.
        assert!(matches!(
            end_of("uk1", &NodeKind::Unikernel, None),
            Some(RecreatedEnd::Domain)
        ));
        assert!(matches!(
            end_of("vm1", &NodeKind::VirtualMachine, None),
            Some(RecreatedEnd::Domain)
        ));
    }

    #[test]
    fn a_started_container_has_its_veths_made_again() {
        // M25.2: a container started from exited has a new network namespace without its
        // veths; resume used to leave it so.
        let image = NodeConfig::get_model(NodeModel::FrrLinux);
        let Some(RecreatedEnd::Container { container, veths }) =
            end_of("ctr1", &NodeKind::Container, Some(&image))
        else {
            panic!("a started container on a link has its ends made again");
        };
        assert_eq!(container, "ctr1-lab1");
        let host_veths: Vec<&str> = veths.iter().map(|v| v.host_veth.as_str()).collect();
        // Its link's end, and its other data interface, disabled, from the manifest's count.
        assert_eq!(host_veths, vec!["tpb1-lab1", "cd3i2-lab1"]);
    }

    #[test]
    fn a_started_container_on_no_link_keeps_its_docker_networks() {
        let image = NodeConfig::get_model(NodeModel::FrrLinux);
        assert!(end_of("ctr9", &NodeKind::Container, Some(&image)).is_none());
    }

    #[test]
    fn a_started_container_that_cannot_be_planned_fails_its_links() {
        assert!(matches!(
            end_of("ctr1", &NodeKind::Container, None),
            Some(RecreatedEnd::Unplanned(_))
        ));
    }
}
