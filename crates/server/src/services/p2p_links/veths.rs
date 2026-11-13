//! A node's links and private bridge memberships from the database, and the veths a container
//! on a P2p link has: one in its network namespace for every data interface.

use std::collections::{HashMap, HashSet};
use std::ops::RangeInclusive;

use anyhow::{Result, anyhow};

use shared::data::{self, DbBridge, DbLink, RecordId};
use shared::konst::{BRIDGE_MEMBER_HOST_VETH_PREFIX, CONTAINER_HOST_VETH_PREFIX};
use shared::util;

/// A data interface of a container on a P2p link: a veth made into the container's network
/// namespace once it has started, with its host end named `host_veth`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContainerVeth {
    pub(crate) host_veth: String,
    pub(crate) host_alias: String,
    pub(crate) interface_idx: u8,
    pub(crate) node_model: data::NodeModel,
    pub(crate) admin_down: bool,
    /// The private bridge the host end joins, for a bridge member.
    pub(crate) bridge: Option<String>,
}

/// The container whose veths are planned.
pub(crate) struct ContainerNode<'a> {
    pub(crate) lab_id: &'a str,
    pub(crate) name: &'a str,
    pub(crate) index: u16,
    pub(crate) model: data::NodeModel,
    pub(crate) record: &'a RecordId,
}

/// The lab's links with an end on `node`.
pub(crate) fn node_links<'a>(links: &'a [DbLink], node: &RecordId) -> Vec<&'a DbLink> {
    links
        .iter()
        .filter(|link| link.node_a == *node || link.node_b == *node)
        .collect()
}

/// The node's interfaces in private bridges, mapped to the bridge's Linux bridge name.
pub(crate) fn node_bridge_interfaces(
    bridges: &[DbBridge],
    node: &RecordId,
) -> HashMap<String, String> {
    let mut interfaces = HashMap::new();
    for bridge in bridges {
        for member in &bridge.members {
            if member.node == *node {
                interfaces.insert(member.interface.clone(), bridge.bridge_name.clone());
            }
        }
    }
    interfaces
}

/// The indices of the node's data interfaces, after its management interface (0) and its
/// reserved interfaces.
pub(crate) fn data_interface_indices(
    node_image: &data::NodeConfig,
    data_interface_count: u8,
    node_name: &str,
) -> Result<RangeInclusive<u8>> {
    let first = node_image
        .interfaces
        .reserved_count
        .checked_add(1)
        .ok_or_else(|| anyhow!("Reserved interface count overflow for node {node_name}"))?;
    let last = node_image
        .interfaces
        .reserved_count
        .checked_add(data_interface_count)
        .ok_or_else(|| anyhow!("Data interface count overflow for node {node_name}"))?;
    Ok(first..=last)
}

/// The veths of a container on a P2p link, one for every data interface: its links, its
/// private bridge memberships, whose host end joins the bridge, and its disabled interfaces,
/// admin down.
pub(crate) fn container_veths(
    node: &ContainerNode<'_>,
    links: &[&DbLink],
    bridge_interfaces: &HashMap<String, String>,
    data_indices: RangeInclusive<u8>,
) -> Result<Vec<ContainerVeth>> {
    let ContainerNode {
        lab_id,
        name: node_name,
        index: node_idx,
        model,
        record,
    } = *node;
    let mut veths = vec![];

    for link in links {
        let ends = [
            (&link.node_a, &link.int_a, &link.tap_a),
            (&link.node_b, &link.int_b, &link.tap_b),
        ];
        for (end, interface, tap) in ends {
            if end != record {
                continue;
            }
            veths.push(ContainerVeth {
                host_veth: tap.clone(),
                host_alias: format!("{lab_id}-p2p-host-{node_name}::{interface}"),
                interface_idx: util::interface_to_idx(&model, interface)?,
                node_model: model,
                admin_down: false,
                bridge: None,
            });
        }
    }

    let mut bridged: Vec<(&String, &String)> = bridge_interfaces.iter().collect();
    bridged.sort();
    for (iface_name, bridge_name) in bridged {
        let iface_idx = util::interface_to_idx(&model, iface_name)?;
        veths.push(ContainerVeth {
            host_veth: format!("{BRIDGE_MEMBER_HOST_VETH_PREFIX}{node_idx}i{iface_idx}-{lab_id}"),
            host_alias: format!("{lab_id}-bridge-host-{node_name}::{iface_name}"),
            interface_idx: iface_idx,
            node_model: model,
            admin_down: false,
            bridge: Some(bridge_name.clone()),
        });
    }

    let connected: HashSet<u8> = veths.iter().map(|v| v.interface_idx).collect();
    for idx in data_indices {
        if connected.contains(&idx) {
            continue;
        }
        veths.push(ContainerVeth {
            host_veth: format!("{CONTAINER_HOST_VETH_PREFIX}{node_idx}i{idx}-{lab_id}"),
            host_alias: format!("{lab_id}-p2p-disabled-host-{node_name}::{idx}"),
            interface_idx: idx,
            node_model: model,
            admin_down: true,
            bridge: None,
        });
    }
    Ok(veths)
}

#[cfg(test)]
mod tests {
    use super::*;

    use shared::data::{BridgeKind, BridgeMember, LinkImpairment, NodeModel};

    fn bridge(index: u16, members: &[(&str, &str)]) -> DbBridge {
        DbBridge {
            id: None,
            index,
            bridge_name: format!("brs{index}-lab"),
            network_name: format!("sherpa-bridge{index}-lab"),
            lab: RecordId::new("lab", "lab"),
            members: members
                .iter()
                .map(|(node, interface)| BridgeMember {
                    node: RecordId::new("node", *node),
                    interface: interface.to_string(),
                })
                .collect(),
        }
    }

    fn link(index: u16, node_a: &str, node_b: &str) -> DbLink {
        DbLink {
            id: None,
            index,
            kind: BridgeKind::P2p,
            node_a: RecordId::new("node", node_a),
            node_b: RecordId::new("node", node_b),
            int_a: "eth1".to_string(),
            int_b: "eth1".to_string(),
            lab: RecordId::new("lab", "lab"),
            tap_a: format!("tap{index}a"),
            tap_b: format!("tap{index}b"),
            impairment_a_to_b: LinkImpairment::default(),
            impairment_b_to_a: LinkImpairment::default(),
        }
    }

    #[test]
    fn test_only_member_interfaces_map_to_their_bridge() {
        // Redeploy used to put every unlinked interface of a bridge's node on that bridge.
        let bridges = vec![
            bridge(0, &[("dev01", "eth1"), ("fr01", "eth1")]),
            bridge(1, &[("dev01", "eth3"), ("fr02", "eth1")]),
        ];
        let interfaces = node_bridge_interfaces(&bridges, &RecordId::new("node", "dev01"));
        assert_eq!(
            interfaces,
            HashMap::from([
                ("eth1".to_string(), "brs0-lab".to_string()),
                ("eth3".to_string(), "brs1-lab".to_string()),
            ])
        );
    }

    #[test]
    fn test_node_in_no_bridge_has_no_bridge_interfaces() {
        let bridges = vec![bridge(0, &[("fr01", "eth1")])];
        assert!(node_bridge_interfaces(&bridges, &RecordId::new("node", "dev01")).is_empty());
    }

    #[test]
    fn a_node_has_the_links_with_an_end_on_it() {
        let links = vec![
            link(0, "dev01", "fr01"),
            link(1, "fr01", "fr02"),
            link(2, "fr02", "dev01"),
        ];
        let indices: Vec<u16> = node_links(&links, &RecordId::new("node", "dev01"))
            .iter()
            .map(|link| link.index)
            .collect();
        assert_eq!(indices, vec![0, 2]);
    }

    #[test]
    fn data_interfaces_follow_the_reserved_ones() {
        let mut image = data::NodeConfig::ubuntu_linux();
        image.interfaces.reserved_count = 2;
        assert_eq!(data_interface_indices(&image, 3, "n1").unwrap(), 3..=5);
    }

    #[test]
    fn a_node_without_data_interfaces_has_none() {
        let mut image = data::NodeConfig::ubuntu_linux();
        image.interfaces.reserved_count = 0;
        assert!(data_interface_indices(&image, 0, "n1").unwrap().is_empty());
    }

    #[test]
    fn an_interface_count_past_the_index_range_is_an_error() {
        let mut image = data::NodeConfig::ubuntu_linux();
        image.interfaces.reserved_count = u8::MAX;
        assert!(data_interface_indices(&image, 1, "n1").is_err());
    }

    #[test]
    fn a_container_has_a_veth_for_every_data_interface() {
        // M25.2: a restarted container needs all of these again, not only its links.
        let record = RecordId::new("node", "ctr1");
        let node = ContainerNode {
            lab_id: "lab1",
            name: "ctr1",
            index: 3,
            model: NodeModel::FrrLinux,
            record: &record,
        };
        let links = [link(0, "vm1", "ctr1")];
        let link_refs: Vec<&DbLink> = links.iter().collect();
        let bridges = HashMap::from([("eth2".to_string(), "brs0-lab1".to_string())]);
        let veths = container_veths(&node, &link_refs, &bridges, 1..=3).unwrap();
        let summary: Vec<(&str, u8, bool, Option<&str>)> = veths
            .iter()
            .map(|v| {
                (
                    v.host_veth.as_str(),
                    v.interface_idx,
                    v.admin_down,
                    v.bridge.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                ("tap0b", 1, false, None),
                ("cb3i2-lab1", 2, false, Some("brs0-lab1")),
                ("cd3i3-lab1", 3, true, None),
            ]
        );
    }
}
