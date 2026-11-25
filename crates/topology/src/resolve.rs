//! Manifest links and bridges resolved against the lab's expanded nodes: each end gets its
//! node's index and model and its interface's index.

use anyhow::{Context, Result};

use shared::konst::BRIDGE_PREFIX;
use shared::util::interface_to_idx;

use crate::{
    Bridge, BridgeDetailed, BridgeLinkDetailed, Link2, LinkDetailed, LinkPlace, NodeExpanded,
    TopologyError,
};

/// The manifest `links` with both ends resolved against `nodes`, numbered from 0.
///
/// # Errors
///
/// Fails when a link is not written as `node::interface`, names a node that is not defined or
/// an interface its node's model does not have, or when there are more links than a `u16`
/// numbers.
pub fn process_manifest_links(
    links: &[Link2],
    nodes: &[NodeExpanded],
) -> Result<Vec<LinkDetailed>> {
    let mut detailed = Vec::with_capacity(links.len());
    for (index, manifest_link) in links.iter().enumerate() {
        let place = LinkPlace::Link {
            src: manifest_link.src.clone(),
            dst: manifest_link.dst.clone(),
        };
        let link =
            manifest_link
                .expand()
                .with_context(|| TopologyError::LinkEndNotNodeInterface {
                    src: manifest_link.src.clone(),
                    dst: manifest_link.dst.clone(),
                })?;
        let link_idx = u16::try_from(index).context("A manifest holds too many links")?;
        let node_a = link_node(nodes, &link.node_a)?;
        let node_b = link_node(nodes, &link.node_b)?;
        detailed.push(LinkDetailed {
            node_a: node_a.name.clone(),
            node_a_idx: node_a.index,
            node_a_model_kind: node_a.model_kind,
            int_a_idx: node_interface_idx(&place, node_a, &link.int_a)?,
            int_a: link.int_a,
            node_b: node_b.name.clone(),
            node_b_idx: node_b.index,
            node_b_model_kind: node_b.model_kind,
            int_b_idx: node_interface_idx(&place, node_b, &link.int_b)?,
            int_b: link.int_b,
            link_idx,
            impairment: link.impairment,
        });
    }
    Ok(detailed)
}

/// The manifest `bridges` of lab `lab_id` with each link resolved against `nodes`, numbered
/// from 0, and named on the host.
///
/// # Errors
///
/// Fails when a bridge link is not written as `node::interface`, names a node that is not
/// defined or an interface its node's model does not have, or when there are more bridges than
/// a `u16` numbers.
pub fn process_manifest_bridges(
    bridges: &[Bridge],
    nodes: &[NodeExpanded],
    lab_id: &str,
) -> Result<Vec<BridgeDetailed>> {
    let mut detailed = Vec::with_capacity(bridges.len());
    for (index, bridge) in bridges.iter().enumerate() {
        let bridge = bridge.parse_links()?;
        let index = u16::try_from(index).context("A manifest holds too many bridges")?;
        let mut links = Vec::with_capacity(bridge.links.len());
        for link in bridge.links {
            let node = nodes
                .iter()
                .find(|node| node.name == link.node)
                .ok_or_else(|| TopologyError::UnknownBridgeNode {
                    node: link.node.clone(),
                })?;
            links.push(BridgeLinkDetailed {
                node_model_kind: node.model_kind,
                interface_index: node_interface_idx(
                    &LinkPlace::Bridge {
                        name: bridge.name.clone(),
                    },
                    node,
                    &link.interface,
                )?,
                node_name: link.node,
                interface_name: link.interface,
            });
        }
        detailed.push(BridgeDetailed {
            bridge_name: format!("{BRIDGE_PREFIX}s{index}-{lab_id}"),
            libvirt_name: format!("sherpa-bridge{index}-{}-{lab_id}", bridge.name),
            manifest_name: bridge.name,
            index,
            links,
        });
    }
    Ok(detailed)
}

/// The node of `nodes` named `name` at one end of a link.
fn link_node<'a>(nodes: &'a [NodeExpanded], name: &str) -> Result<&'a NodeExpanded> {
    nodes.iter().find(|node| node.name == name).ok_or_else(|| {
        TopologyError::UnknownLinkNode {
            node: name.to_owned(),
        }
        .into()
    })
}

/// The index of `node`'s interface named `interface`, at the link or bridge `place` names.
fn node_interface_idx(place: &LinkPlace, node: &NodeExpanded, interface: &str) -> Result<u8> {
    interface_to_idx(node.model_kind, interface).with_context(|| TopologyError::NoSuchInterface {
        at: place.clone(),
        node: node.name.clone(),
        model_kind: node.model_kind,
        interface: interface.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use shared::data::{ModelKind, NodeModel};

    use super::*;
    use crate::Node;

    fn nodes(names: &[&str]) -> Vec<NodeExpanded> {
        (1..)
            .zip(names)
            .map(|(index, name)| {
                NodeExpanded::new(
                    index,
                    &Node {
                        name: (*name).to_string(),
                        model: NodeModel::UbuntuLinux,
                        ..Default::default()
                    },
                )
                .unwrap()
            })
            .collect()
    }

    fn link(src: &str, dst: &str) -> Link2 {
        Link2 {
            src: src.to_string(),
            dst: dst.to_string(),
            impairment: None,
        }
    }

    fn bridge(name: &str, links: &[&str]) -> Bridge {
        Bridge {
            name: name.to_string(),
            links: links.iter().map(ToString::to_string).collect(),
        }
    }

    #[test]
    fn manifest_links_001_a_links_two_ends_resolve_to_their_nodes_and_interfaces() {
        let result = process_manifest_links(
            &[link("dev01::eth1", "dev02::eth3")],
            &nodes(&["dev01", "dev02"]),
        )
        .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].node_a, "dev01");
        assert_eq!(result[0].node_a_idx, 1);
        assert_eq!(result[0].int_a, "eth1");
        assert_eq!(result[0].int_a_idx, 1);
        assert_eq!(result[0].node_b, "dev02");
        assert_eq!(result[0].node_b_idx, 2);
        assert_eq!(result[0].int_b, "eth3");
        assert_eq!(result[0].int_b_idx, 3);
    }

    #[test]
    fn manifest_links_002_links_are_numbered_from_0_in_manifest_order() {
        let links = [
            link("dev01::eth1", "dev02::eth1"),
            link("dev02::eth2", "dev03::eth1"),
        ];
        let result = process_manifest_links(&links, &nodes(&["dev01", "dev02", "dev03"])).unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].link_idx, 0);
        assert_eq!(result[1].link_idx, 1);
    }

    #[test]
    fn manifest_links_003_an_end_naming_an_unknown_node_is_refused_at_either_end() {
        for (src, dst, unknown) in [
            ("dev0l::eth1", "dev02::eth1", "dev0l"),
            ("dev01::eth1", "dev0x::eth1", "dev0x"),
            ("dev0l::eth1", "dev0x::eth1", "dev0l"),
        ] {
            let error =
                process_manifest_links(&[link(src, dst)], &nodes(&["dev01", "dev02"])).unwrap_err();
            assert_eq!(
                error.downcast_ref::<TopologyError>(),
                Some(&TopologyError::UnknownLinkNode {
                    node: unknown.to_owned(),
                })
            );
        }
    }

    #[test]
    fn manifest_links_004_an_end_that_is_not_written_node_interface_is_refused_naming_it() {
        let error = process_manifest_links(
            &[link("dev01:eth1", "dev02::eth1")],
            &nodes(&["dev01", "dev02"]),
        )
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::LinkEndNotNodeInterface {
                src: "dev01:eth1".to_owned(),
                dst: "dev02::eth1".to_owned(),
            })
        );

        let error = process_manifest_bridges(
            &[bridge("lan", &["dev01::eth1", "dev02:eth1"])],
            &nodes(&["dev01", "dev02"]),
            "lab1",
        )
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::BridgeLinkNotNodeInterface {
                bridge: "lan".to_owned(),
                link: "dev02:eth1".to_owned(),
            })
        );
    }

    #[test]
    fn manifest_links_005_an_interface_the_nodes_model_does_not_have_is_refused_naming_it() {
        let error = process_manifest_links(
            &[link("dev01::eth1", "dev02::gig4")],
            &nodes(&["dev01", "dev02"]),
        )
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::NoSuchInterface {
                at: LinkPlace::Link {
                    src: "dev01::eth1".to_owned(),
                    dst: "dev02::gig4".to_owned(),
                },
                node: "dev02".to_owned(),
                model_kind: ModelKind::default_for(NodeModel::UbuntuLinux),
                interface: "gig4".to_owned(),
            })
        );

        let error = process_manifest_bridges(
            &[bridge("lan", &["dev01::gig4"])],
            &nodes(&["dev01"]),
            "lab1",
        )
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::NoSuchInterface {
                at: LinkPlace::Bridge {
                    name: "lan".to_owned(),
                },
                node: "dev01".to_owned(),
                model_kind: ModelKind::default_for(NodeModel::UbuntuLinux),
                interface: "gig4".to_owned(),
            })
        );
    }

    #[test]
    fn manifest_links_010_a_bridge_resolves_its_links_and_is_named_by_its_place_and_the_lab_id() {
        let result = process_manifest_bridges(
            &[bridge("mgmt", &["dev01::eth1", "dev02::eth2"])],
            &nodes(&["dev01", "dev02"]),
            "abc123",
        )
        .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].manifest_name, "mgmt");
        assert_eq!(result[0].bridge_name, "brs0-abc123");
        assert_eq!(result[0].libvirt_name, "sherpa-bridge0-mgmt-abc123");
        assert_eq!(result[0].index, 0);
        assert_eq!(result[0].links.len(), 2);
        assert_eq!(result[0].links[1].node_name, "dev02");
        assert_eq!(
            result[0].links[1].node_model_kind,
            ModelKind::default_for(NodeModel::UbuntuLinux)
        );
        assert_eq!(result[0].links[1].interface_name, "eth2");
        assert_eq!(result[0].links[1].interface_index, 2);
    }

    #[test]
    fn manifest_links_011_a_bridge_link_naming_an_unknown_node_is_refused() {
        let error = process_manifest_bridges(
            &[bridge("lan", &["dev01::eth1", "dev0l::eth1"])],
            &nodes(&["dev01", "dev02"]),
            "lab1",
        )
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::UnknownBridgeNode {
                node: "dev0l".to_owned(),
            })
        );
    }
}
