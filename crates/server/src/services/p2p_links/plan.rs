//! How a node's link ends are made again, planned from the database and the lab's stored
//! definition: what a cold boot by resume and the scanner's repair hand to re-attach.

use std::collections::HashMap;

use anyhow::{Result, anyhow};

use super::reattach::{LabLinks, RecreatedEnd};
use super::veths::{
    ContainerNode, container_veths, data_interface_indices, node_bridge_interfaces, node_links,
};

use shared::data::{DbBridge, DbLink, DbNode, ModelKind, NodeConfig, NodeKind, RecordId};
use shared::validators::effective_data_interface_count;
use topology::Manifest;

use crate::services::error::Unexpected;

/// A node whose link ends are planned.
pub(crate) struct PlannedNode<'a> {
    pub(crate) lab_id: &'a str,
    pub(crate) node: &'a DbNode,
    pub(crate) node_id: &'a RecordId,
    pub(crate) kind: &'a NodeKind,
    pub(crate) image: Option<&'a NodeConfig>,
}

/// How the node's link ends are made again, or `None` when it has no ends to re-attach.
///
/// libvirt makes a VM's or a unikernel's taps when it boots the domain. A container started
/// from exited, or restarted, has a new network namespace with none of its veths, so each of
/// its data interfaces is planned again, as redeploy does, with the interface count of the lab's
/// stored definition (`definition`), or the image's without one.
pub(crate) fn recreated_end(
    node: &PlannedNode<'_>,
    definition: Option<&Manifest>,
    lab_links: &LabLinks,
    bridges: Result<&[DbBridge], &String>,
) -> Option<RecreatedEnd> {
    match node.kind {
        NodeKind::VirtualMachine | NodeKind::Unikernel => Some(RecreatedEnd::Domain),
        // Cloud Hypervisor makes a microVM's link ends as taps at boot, as libvirt does.
        NodeKind::Container
            if node
                .image
                .and_then(|image| image.model_kind().ok())
                .is_some_and(ModelKind::runs_in_microvm) =>
        {
            Some(RecreatedEnd::Domain)
        }
        NodeKind::Container => {
            let links = node_links(lab_links.links(), node.node_id);
            let bridge_interfaces =
                bridges.map(|bridges| node_bridge_interfaces(bridges, node.node_id));
            if links.is_empty() && bridge_interfaces.as_ref().is_ok_and(HashMap::is_empty) {
                return None;
            }
            Some(
                container_end(node, definition, &links, bridge_interfaces).unwrap_or_else(
                    |error| {
                        RecreatedEnd::Unplanned(
                            Unexpected::log(&error, "Failed to plan a container's link ends")
                                .to_string(),
                        )
                    },
                ),
            )
        }
    }
}

/// The container's veths, planned from its links, bridge interfaces and data interface count.
fn container_end(
    node: &PlannedNode<'_>,
    definition: Option<&Manifest>,
    links: &[&DbLink],
    bridge_interfaces: Result<HashMap<String, String>, &String>,
) -> Result<RecreatedEnd> {
    let name = node.node.name.as_str();
    let image = node
        .image
        .ok_or_else(|| anyhow!("Node image of {name} not found"))?;
    let bridge_interfaces =
        bridge_interfaces.map_err(|error| anyhow!("Failed to list the lab's bridges: {error}"))?;
    let defined_count = definition
        .and_then(|definition| definition.nodes.iter().find(|n| n.name == name))
        .and_then(|n| n.interfaces.data_count);
    let count = effective_data_interface_count(name, defined_count, image)?;
    let veths = container_veths(
        &ContainerNode {
            lab_id: node.lab_id,
            name,
            index: node.node.index,
            model_kind: image.model_kind()?,
            record: node.node_id,
        },
        links,
        &bridge_interfaces,
        data_interface_indices(image, count, name)?,
    )?;
    Ok(RecreatedEnd::Container {
        container: format!("{name}-{}", node.lab_id),
        veths,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::services::lab_definition::stored_definition;
    use crate::services::lab_definition::tests::{lab_with, store};
    use shared::data::{ModelKind, ReadyCheck};

    use std::collections::HashMap;

    use super::super::fake::FakeHost;
    use super::super::reattach::reattach_node_links;
    use shared::data::{BridgeKind, BridgeMember, LinkImpairment, NodeModel, NodeState};

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
            ssh_host_key: None,
            ready_check: ReadyCheck::default(),
            ssh_username: None,
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

    /// The lab's stored definition: `ctr1` with two data interfaces.
    pub(crate) async fn definition() -> Option<Manifest> {
        let (db, lab) = lab_with(&[("ctr1", NodeModel::FrrLinux)]).await;
        store(
            &db,
            &lab,
            r#"name = "l"
[[nodes]]
name = "ctr1"
model = "frr_linux"
[nodes.interfaces]
data_count = 2
"#,
        )
        .await;
        stored_definition(&db, lab.id.as_ref().unwrap(), "lab1").await
    }

    pub(crate) fn end_of(
        name: &str,
        kind: NodeKind,
        image: Option<&NodeConfig>,
        definition: Option<&Manifest>,
    ) -> Option<RecreatedEnd> {
        let node = db_node(name);
        let node_id = RecordId::new("node", name);
        let bridges: Vec<DbBridge> = vec![];
        recreated_end(
            &PlannedNode {
                lab_id: "lab1",
                node: &node,
                node_id: &node_id,
                kind: &kind,
                image,
            },
            definition,
            &linked_lab(),
            Ok(&bridges),
        )
    }

    /// The host veths planned for a started `ctr1`.
    pub(crate) fn ctr1_host_veths(definition: Option<&Manifest>) -> Vec<String> {
        let image = ModelKind::default_for(NodeModel::FrrLinux).node_config();
        let Some(RecreatedEnd::Container { container, veths }) =
            end_of("ctr1", NodeKind::Container, Some(&image), definition)
        else {
            panic!("a started container on a link has its ends made again");
        };
        assert_eq!(container, "ctr1-lab1");
        veths.into_iter().map(|v| v.host_veth).collect()
    }

    #[test]
    fn privileged_microvm_020_a_microvm_containers_link_ends_are_re_attached_as_a_domains() {
        let image = ModelKind::new(NodeModel::AristaEos, Some(NodeKind::Container))
            .expect("model and kind")
            .node_config();
        assert!(matches!(
            end_of("ctr1", NodeKind::Container, Some(&image), None),
            Some(RecreatedEnd::Domain)
        ));
    }

    #[tokio::test]
    async fn link_reattach_008_a_container_of_a_lab_with_no_stored_manifest_has_its_image_count() {
        let (db, lab) = lab_with(&[("ctr1", NodeModel::FrrLinux)]).await;
        let definition = stored_definition(&db, lab.id.as_ref().unwrap(), "lab1").await;
        assert_eq!(definition, None);
        let image = ModelKind::default_for(NodeModel::FrrLinux).node_config();
        let veths = ctr1_host_veths(definition.as_ref());
        assert_eq!(veths.len(), usize::from(image.interfaces.data_count));
        assert_eq!(veths[0], "tpb1-lab1");
    }

    #[tokio::test]
    async fn link_reattach_009_a_started_container_on_no_link_or_bridge_keeps_its_docker_networks()
    {
        let definition = definition().await;
        let image = ModelKind::default_for(NodeModel::FrrLinux).node_config();
        assert!(
            end_of(
                "ctr9",
                NodeKind::Container,
                Some(&image),
                definition.as_ref()
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn veth_checksums_012_an_iol_container_on_a_bridge_and_no_link_gets_veths_with_offload_off()
     {
        let image = ModelKind::default_for(NodeModel::CiscoIol).node_config();
        let node = db_node("r1");
        let node_id = RecordId::new("node", "r1");
        let bridges = vec![DbBridge {
            id: None,
            index: 0,
            bridge_name: "brs0-lab1".to_string(),
            network_name: "sherpa-bridge0-private-lab1".to_string(),
            lab: RecordId::new("lab", "x"),
            members: vec![BridgeMember {
                node: node_id.clone(),
                interface: "Ethernet0/1".to_string(),
            }],
        }];
        let lab = LabLinks::new("lab1", vec![], &[], HashMap::new());
        let end = recreated_end(
            &PlannedNode {
                lab_id: "lab1",
                node: &node,
                node_id: &node_id,
                kind: &NodeKind::Container,
                image: Some(&image),
            },
            None,
            &lab,
            Ok(&bridges),
        )
        .expect("a container on a bridge has its interfaces made again");

        let host = FakeHost::default();
        reattach_node_links(&host, &lab, &node_id, end).await;
        assert_eq!(
            host.bridge_ports
                .borrow()
                .get("cb3i1-lab1")
                .map(String::as_str),
            Some("brs0-lab1")
        );
        let made: Vec<String> = host.interfaces.borrow().keys().cloned().collect();
        assert_eq!(
            made.len(),
            usize::from(image.interfaces.data_count),
            "{made:?}"
        );
        for iface in &made {
            let index = host.index(iface).unwrap();
            assert!(!host.offload_on.borrow().contains(&index), "{iface}");
        }

        let frr = ModelKind::default_for(NodeModel::FrrLinux).node_config();
        let host = FakeHost::default();
        let end = recreated_end(
            &PlannedNode {
                lab_id: "lab1",
                node: &node,
                node_id: &node_id,
                kind: &NodeKind::Container,
                image: Some(&frr),
            },
            None,
            &lab,
            Ok(&[DbBridge {
                members: vec![BridgeMember {
                    node: node_id.clone(),
                    interface: "eth1".to_string(),
                }],
                ..bridges[0].clone()
            }]),
        )
        .unwrap();
        reattach_node_links(&host, &lab, &node_id, end).await;
        let index = host.index("cb3i1-lab1").unwrap();
        assert!(host.offload_on.borrow().contains(&index));
    }
}
