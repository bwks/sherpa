//! A node's links and private bridge memberships from the database, and the veths a container
//! on a P2p link or a private bridge has: one in its network namespace for every data interface.

use std::collections::{HashMap, HashSet};
use std::ops::RangeInclusive;

use anyhow::{Result, anyhow};

use shared::data::{self, DbBridge, DbLink, RecordId};
use shared::util;

use crate::konst::{BRIDGE_MEMBER_HOST_VETH_PREFIX, CONTAINER_HOST_VETH_PREFIX};

/// A data interface of a container on a P2p link or a private bridge: a veth made into the
/// container's network namespace once it has started, with its host end named `host_veth`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContainerVeth {
    pub(crate) host_veth: String,
    pub(crate) host_alias: String,
    pub(crate) interface_idx: u8,
    pub(crate) model_kind: data::ModelKind,
    pub(crate) admin_down: bool,
    /// The private bridge the host end joins, for a bridge member.
    pub(crate) bridge: Option<String>,
}

/// The container whose veths are planned.
pub(crate) struct ContainerNode<'a> {
    pub(crate) lab_id: &'a str,
    pub(crate) name: &'a str,
    pub(crate) index: u16,
    pub(crate) model_kind: data::ModelKind,
    pub(crate) record: &'a RecordId,
}

/// The host end of a container's private bridge member veth, as the scanner checks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BridgeEnd {
    pub(crate) node: RecordId,
    pub(crate) host_veth: String,
}

/// The host end name of the veth of interface `iface_idx` of node `node_idx` in a private bridge.
pub(crate) fn bridge_member_host_veth(lab_id: &str, node_idx: u16, iface_idx: u8) -> String {
    format!("{BRIDGE_MEMBER_HOST_VETH_PREFIX}{node_idx}i{iface_idx}-{lab_id}")
}

/// The host ends of the private bridge member veths of container `node`.
pub(crate) fn bridge_ends(
    node: &ContainerNode<'_>,
    bridges: &[DbBridge],
) -> Result<Vec<BridgeEnd>> {
    let mut ends = vec![];
    for iface_name in node_bridge_interfaces(bridges, node.record).keys() {
        let iface_idx = util::interface_to_idx(node.model_kind, iface_name)?;
        ends.push(BridgeEnd {
            node: node.record.clone(),
            host_veth: bridge_member_host_veth(node.lab_id, node.index, iface_idx),
        });
    }
    ends.sort_by(|a, b| a.host_veth.cmp(&b.host_veth));
    Ok(ends)
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

/// The veths of a container on a P2p link or a private bridge, one for every data interface: its
/// links, its private bridge memberships, whose host end joins the bridge, and its disabled
/// interfaces, admin down.
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
        model_kind,
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
                interface_idx: util::interface_to_idx(model_kind, interface)?,
                model_kind,
                admin_down: false,
                bridge: None,
            });
        }
    }

    let mut bridged: Vec<(&String, &String)> = bridge_interfaces.iter().collect();
    bridged.sort();
    for (iface_name, bridge_name) in bridged {
        let iface_idx = util::interface_to_idx(model_kind, iface_name)?;
        veths.push(ContainerVeth {
            host_veth: bridge_member_host_veth(lab_id, node_idx, iface_idx),
            host_alias: format!("{lab_id}-bridge-host-{node_name}::{iface_name}"),
            interface_idx: iface_idx,
            model_kind,
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
            model_kind,
            admin_down: true,
            bridge: None,
        });
    }
    Ok(veths)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    use shared::data::{BridgeKind, BridgeMember, LinkImpairment};
    use shared::validators::effective_data_interface_count;

    pub(crate) fn bridge(index: u16, members: &[(&str, &str)]) -> DbBridge {
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

    pub(crate) fn link(index: u16, node_a: &str, node_b: &str) -> DbLink {
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
    fn node_interfaces_008_data_interfaces_number_as_the_manifest_or_image_says_after_reserved() {
        let mut image = data::NodeConfig::ubuntu_linux();
        image.interfaces.reserved_count = 2;
        assert_eq!(data_interface_indices(&image, 3, "n1").unwrap(), 3..=5);

        let mut image = data::NodeConfig::ubuntu_linux();
        image.interfaces.reserved_count = 0;
        assert!(data_interface_indices(&image, 0, "n1").unwrap().is_empty());

        let mut image = data::NodeConfig::ubuntu_linux();
        image.interfaces.reserved_count = u8::MAX;
        assert!(data_interface_indices(&image, 1, "n1").is_err());

        let node_image = data::NodeConfig::ubuntu_linux();
        let count = effective_data_interface_count("dev01", None, &node_image).unwrap();
        assert_eq!(count, 1);

        let node_image = data::NodeConfig::ubuntu_linux();
        let count = effective_data_interface_count("dev01", Some(4), &node_image).unwrap();
        assert_eq!(count, 4);
    }
}
