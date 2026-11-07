//! The node's P2p links and private bridges, and re-attaching the links' eBPF redirects.

use std::collections::HashMap;

use anyhow::{Context, Result};
use tracing::instrument;

use crate::services::progress::ProgressSender;

use shared::data::{DbBridge, DbLink, RecordId, StatusKind};

/// The lab's links with an end on `node`.
pub(super) fn node_links<'a>(links: &'a [DbLink], node: &RecordId) -> Vec<&'a DbLink> {
    links
        .iter()
        .filter(|link| link.node_a == *node || link.node_b == *node)
        .collect()
}

/// The node's interfaces in private bridges, mapped to the bridge's Linux bridge name.
pub(super) fn node_bridge_interfaces(
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

/// Attach the eBPF redirect and the impairments again on each of the node's links, once its
/// rebuilt end exists.
#[instrument(skip_all, fields(%lab_id), level = "debug")]
pub(super) async fn reattach_p2p_redirects(
    lab_id: &str,
    links: &[&DbLink],
    progress: &ProgressSender,
) -> Result<()> {
    if links.is_empty() {
        return Ok(());
    }
    let _ = progress.send_status(
        format!("Re-attaching eBPF redirect for {} P2p links", links.len()),
        StatusKind::Progress,
    );

    for link in links {
        let ifindex_a = network::get_ifindex(&link.tap_a)
            .await
            .context(format!("failed to get ifindex for {}", link.tap_a))?;
        let ifindex_b = network::get_ifindex(&link.tap_b)
            .await
            .context(format!("failed to get ifindex for {}", link.tap_b))?;

        network::attach_p2p_redirect(&link.tap_a, ifindex_b)
            .context(format!("failed to attach eBPF redirect on {}", link.tap_a))?;
        network::attach_p2p_redirect(&link.tap_b, ifindex_a)
            .context(format!("failed to attach eBPF redirect on {}", link.tap_b))?;

        network::apply_link_netem(
            ifindex_a,
            ifindex_b,
            &link.impairment_a_to_b,
            &link.impairment_b_to_a,
        )
        .await?;

        tracing::info!(
            lab_id = %lab_id,
            tap_a = %link.tap_a,
            tap_b = %link.tap_b,
            "Re-attached eBPF P2p redirect (redeploy)"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use shared::data::{BridgeKind, BridgeMember, LinkImpairment};

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
}
