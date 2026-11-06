//! Manifest links and bridges resolved against the lab's expanded nodes: each end gets its
//! node's index and model and its interface's index.

use anyhow::{Context, Result};

use shared::konst::BRIDGE_PREFIX;
use shared::util::interface_to_idx;

use crate::{Bridge, BridgeDetailed, BridgeLinkDetailed, Link2, LinkDetailed, NodeExpanded};

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
    for (index, link) in links.iter().enumerate() {
        let link = link.expand()?;
        let link_idx = u16::try_from(index).context("A manifest holds too many links")?;
        let node_a = link_node(nodes, &link.node_a)?;
        let node_b = link_node(nodes, &link.node_b)?;
        detailed.push(LinkDetailed {
            node_a: node_a.name.clone(),
            node_a_idx: node_a.index,
            node_a_model: node_a.model,
            int_a_idx: interface_to_idx(&node_a.model, &link.int_a)?,
            int_a: link.int_a,
            node_b: node_b.name.clone(),
            node_b_idx: node_b.index,
            node_b_model: node_b.model,
            int_b_idx: interface_to_idx(&node_b.model, &link.int_b)?,
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
                .with_context(|| {
                    format!(
                        "Manifest bridge - device '{}' defined in bridges, not defined in devices",
                        link.node
                    )
                })?;
            links.push(BridgeLinkDetailed {
                node_model: node.model,
                interface_index: interface_to_idx(&node.model, &link.interface)?,
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
    nodes
        .iter()
        .find(|node| node.name == name)
        .with_context(|| {
            format!("Manifest link - '{name}' defined in links, not defined in devices")
        })
}

#[cfg(test)]
mod tests {
    use shared::data::NodeModel;

    use super::*;
    use crate::Node;
    use crate::link::ManifestImpairment;

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
            })
            .collect()
    }

    fn link(src: &str, dst: &str) -> Link2 {
        Link2 {
            src: src.to_string(),
            dst: dst.to_string(),
            p2p: None,
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
    fn a_link_resolves_both_ends() {
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
    fn links_are_numbered_in_order() {
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
    fn no_links_resolve_to_none() {
        assert!(
            process_manifest_links(&[], &nodes(&["dev01"]))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_link_keeps_its_impairment() {
        let mut impaired = link("dev01::eth1", "dev02::eth1");
        impaired.impairment = Some(ManifestImpairment {
            delay: Some(10),
            ..Default::default()
        });
        let result = process_manifest_links(&[impaired], &nodes(&["dev01", "dev02"])).unwrap();
        assert_eq!(result[0].impairment.as_ref().unwrap().delay, Some(10));
    }

    #[test]
    fn a_link_between_two_interfaces_of_one_node_resolves_both_ends() {
        let result =
            process_manifest_links(&[link("dev01::eth1", "dev01::eth2")], &nodes(&["dev01"]))
                .unwrap();
        assert_eq!(result[0].node_a, "dev01");
        assert_eq!(result[0].int_a_idx, 1);
        assert_eq!(result[0].node_b, "dev01");
        assert_eq!(result[0].node_b_idx, 1);
        assert_eq!(result[0].int_b, "eth2");
        assert_eq!(result[0].int_b_idx, 2);
    }

    #[test]
    fn a_link_naming_an_unknown_node_is_refused() {
        for (src, dst, unknown) in [
            ("dev0l::eth1", "dev02::eth1", "dev0l"),
            ("dev01::eth1", "dev0x::eth1", "dev0x"),
            ("dev0l::eth1", "dev0x::eth1", "dev0l"),
        ] {
            let error =
                process_manifest_links(&[link(src, dst)], &nodes(&["dev01", "dev02"])).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("Manifest link - '{unknown}' defined in links, not defined in devices")
            );
        }
    }

    #[test]
    fn a_bridge_is_named_on_the_host_and_resolves_its_links() {
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
        assert_eq!(result[0].links[1].node_model, NodeModel::UbuntuLinux);
        assert_eq!(result[0].links[1].interface_name, "eth2");
        assert_eq!(result[0].links[1].interface_index, 2);
    }

    #[test]
    fn no_bridges_resolve_to_none() {
        assert!(
            process_manifest_bridges(&[], &nodes(&[]), "lab1")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_bridge_naming_an_unknown_node_is_refused() {
        let error = process_manifest_bridges(
            &[bridge("lan", &["dev01::eth1", "dev0l::eth1"])],
            &nodes(&["dev01", "dev02"]),
            "lab1",
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Manifest bridge - device 'dev0l' defined in bridges, not defined in devices"
        );
    }
}
