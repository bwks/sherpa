//! Point-to-point links and the P2p container veths, and attaching their eBPF redirects.

use std::collections::HashSet;

use anyhow::{Context, Result, anyhow};
use tracing::instrument;

use super::context::LabBuild;

use shared::data;
use shared::data::{LinkImpairment, RecordId, StatusKind};
use shared::konst::{CONTAINER_HOST_VETH_PREFIX, TAP_PREFIX};

/// A data interface of a P2p container: a veth created into the container's netns once it has
/// started, with its host end named `host_veth`.
pub(super) struct P2pContainerVeth {
    pub(super) node_name: String,
    pub(super) host_veth: String,
    pub(super) host_alias: String,
    pub(super) interface_idx: u8,
    pub(super) node_model: data::NodeModel,
    pub(super) admin_down: bool,
    /// The private bridge the host end joins, for a bridge member.
    pub(super) bridge: Option<String>,
}

/// Attach the eBPF redirect, and any impairment, on each P2p link not yet in `attached` whose
/// ends both exist, by name in `ready`. Returns how many links it attached.
#[instrument(skip_all, fields(%lab_id), level = "debug")]
pub(super) async fn attach_ready_p2p_links(
    lab_id: &str,
    p2p_links: &[&data::LabLinkData],
    links_detailed: &[topology::LinkDetailed],
    ready: &HashSet<String>,
    attached: &mut HashSet<u16>,
) -> Result<usize> {
    let mut count = 0;
    for link_data in p2p_links {
        if attached.contains(&link_data.index)
            || !ready.contains(&link_data.tap_a)
            || !ready.contains(&link_data.tap_b)
        {
            continue;
        }
        let tap_a = &link_data.tap_a;
        let tap_b = &link_data.tap_b;

        let ifindex_a = network::get_ifindex(tap_a)
            .await
            .context(format!("failed to get ifindex for {tap_a}"))?;
        let ifindex_b = network::get_ifindex(tap_b)
            .await
            .context(format!("failed to get ifindex for {tap_b}"))?;

        network::attach_p2p_redirect(tap_a, ifindex_b)
            .context(format!("failed to attach eBPF redirect on {tap_a}"))?;
        network::attach_p2p_redirect(tap_b, ifindex_a)
            .context(format!("failed to attach eBPF redirect on {tap_b}"))?;

        // Apply link impairment if configured
        let link_detail = links_detailed
            .iter()
            .find(|l| l.link_idx == link_data.index);
        if let Some(ld) = link_detail
            && let Some(ref impairment_cfg) = ld.impairment
        {
            let impairment = LinkImpairment::try_from(impairment_cfg)?;
            network::apply_link_netem(ifindex_a, ifindex_b, &impairment, &impairment).await?;
        }

        tracing::info!(
            lab_id = %lab_id,
            tap_a = %tap_a,
            tap_b = %tap_b,
            ifindex_a = ifindex_a,
            ifindex_b = ifindex_b,
            "Attached eBPF P2p redirect"
        );
        attached.insert(link_data.index);
        count += 1;
    }
    Ok(count)
}

pub(super) fn find_interface_link(
    node_name: &str,
    interface_name: &str,
    links_detailed: &[topology::LinkDetailed],
) -> Option<data::NodeInterface> {
    let mut interface_data = None;
    for link in links_detailed {
        if link.node_a == node_name && link.int_a == *interface_name {
            interface_data = Some(data::NodeInterface::Peer(data::PeerInterface {
                link_index: link.link_idx,
                this_node: link.node_a.clone(),
                this_node_index: link.node_a_idx,
                this_interface: link.int_a.clone(),
                this_interface_index: link.int_a_idx,
                this_side: data::PeerSide::A,
                peer_node: link.node_b.clone(),
                peer_node_index: link.node_b_idx,
                peer_interface: link.int_b.clone(),
                peer_interface_index: link.int_b_idx,
                peer_side: data::PeerSide::B,
            }))
        } else if link.node_b == node_name && link.int_b == *interface_name {
            interface_data = Some(data::NodeInterface::Peer(data::PeerInterface {
                link_index: link.link_idx,
                this_node: link.node_b.clone(),
                this_node_index: link.node_b_idx,
                this_interface: link.int_b.clone(),
                this_interface_index: link.int_b_idx,
                this_side: data::PeerSide::B,
                peer_node: link.node_a.clone(),
                peer_node_index: link.node_a_idx,
                peer_interface: link.int_a.clone(),
                peer_interface_index: link.int_a_idx,
                peer_side: data::PeerSide::A,
            }))
        }
    }
    interface_data
}

pub(super) fn find_bridge_interface(
    node_name: &str,
    interface_name: &str,
    bridge_connections: &[topology::BridgeDetailed],
) -> Option<data::NodeInterface> {
    let mut interface_data = None;
    for bridge in bridge_connections.iter() {
        for link in bridge.links.iter() {
            if link.node_name == node_name && link.interface_name == *interface_name {
                interface_data = Some(data::NodeInterface::Bridge(data::BridgeInterface {
                    name: bridge.bridge_name.clone(),
                }))
            }
        }
    }
    interface_data
}

/// The lab's P2p links as recorded, and the veths of the containers on them.
pub(super) struct LinkRecords {
    pub(super) lab_link_data: Vec<data::LabLinkData>,
    pub(super) p2p_container_veths: Vec<P2pContainerVeth>,
    /// Containers on a P2p link. They take every data interface as a veth in their network
    /// namespace, so they get no Docker data networks.
    pub(super) p2p_container_names: HashSet<String>,
}

/// Record every P2p link with the manifest's impairment, and plan the veths of the containers
/// on them, including one for each of those containers' disabled interfaces. Creates no
/// interfaces: libvirt makes VM taps, and container veths are made once the containers start.
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn create_links(
    build: &LabBuild<'_>,
    lab_record_id: &RecordId,
    links_detailed: &[topology::LinkDetailed],
    lab_node_data: &[data::LabNodeData],
    node_setup_data: &[data::NodeSetupData],
) -> Result<LinkRecords> {
    let LabBuild {
        lab_id,
        db,
        progress,
        ..
    } = *build;

    tracing::info!(
        lab_id = %lab_id,
        link_count = links_detailed.len(),
        "Creating point-to-point links"
    );

    let mut lab_link_data = vec![];

    let mut p2p_container_veths: Vec<P2pContainerVeth> = vec![];
    // Track which container nodes participate in P2p links.
    // These containers handle ALL data interfaces via veth+netns, not Docker networks.
    let mut p2p_container_names: HashSet<String> = HashSet::new();

    for (idx, link) in links_detailed.iter().enumerate() {
        let node_a = lab_node_data
            .iter()
            .find(|n| n.name == link.node_a)
            .ok_or_else(|| anyhow!("Node not found: {}", link.node_a))?;

        let node_b = lab_node_data
            .iter()
            .find(|n| n.name == link.node_b)
            .ok_or_else(|| anyhow!("Node not found: {}", link.node_b))?;

        let link_kind = data::BridgeKind::P2p;

        let tap_a = format!("{}a{}-{}", TAP_PREFIX, link.link_idx, lab_id);
        let tap_b = format!("{}b{}-{}", TAP_PREFIX, link.link_idx, lab_id);

        // The manifest's impairment applies to both directions.
        let impairment = link
            .impairment
            .as_ref()
            .map(LinkImpairment::try_from)
            .transpose()?
            .unwrap_or_default();
        db::create_link(
            db,
            data::DbLink {
                id: None,
                index: link.link_idx,
                kind: link_kind.clone(),
                node_a: db::get_node_id(&node_a.record)?,
                node_b: db::get_node_id(&node_b.record)?,
                int_a: link.int_a.clone(),
                int_b: link.int_b.clone(),
                lab: lab_record_id.clone(),
                tap_a: tap_a.clone(),
                tap_b: tap_b.clone(),
                impairment_a_to_b: impairment,
                impairment_b_to_a: impairment,
            },
        )
        .await?;

        lab_link_data.push(data::LabLinkData {
            index: link.link_idx,
            kind: link_kind.clone(),
            node_a: node_a.record.clone(),
            node_b: node_b.record.clone(),
            int_a: link.int_a.clone(),
            int_b: link.int_b.clone(),
            tap_a: tap_a.clone(),
            tap_b: tap_b.clone(),
        });

        let _ = progress.send_status(
            format!(
                "Creating link #{} ({}) - {}::{} <-> {}::{}",
                idx, link_kind, link.node_a, link.int_a, link.node_b, link.int_b
            ),
            StatusKind::Progress,
        );

        tracing::info!(
            lab_id = %lab_id,
            link_num = idx,
            link_kind = %link_kind,
            node_a = %link.node_a,
            int_a = %link.int_a,
            node_b = %link.node_b,
            int_b = %link.int_b,
            "Creating point-to-point link"
        );

        // P2p link: for VMs, libvirt creates the tap devices via type='ethernet'.
        // A container end is a veth created into the container's netns once it has
        // started, its host end named as the tap. eBPF redirect is attached once both
        // ends exist.

        if node_a.kind == data::NodeKind::Container {
            p2p_container_veths.push(P2pContainerVeth {
                node_name: link.node_a.clone(),
                host_veth: tap_a.clone(),
                host_alias: format!("{}-p2p-host-{}::{}", lab_id, link.node_a, link.int_a),
                interface_idx: link.int_a_idx,
                node_model: link.node_a_model,
                admin_down: false,
                bridge: None,
            });
            p2p_container_names.insert(link.node_a.clone());
        }

        if node_b.kind == data::NodeKind::Container {
            p2p_container_veths.push(P2pContainerVeth {
                node_name: link.node_b.clone(),
                host_veth: tap_b.clone(),
                host_alias: format!("{}-p2p-host-{}::{}", lab_id, link.node_b, link.int_b),
                interface_idx: link.int_b_idx,
                node_model: link.node_b_model,
                admin_down: false,
                bridge: None,
            });
            p2p_container_names.insert(link.node_b.clone());
        }

        tracing::info!(
            lab_id = %lab_id,
            tap_a = %tap_a,
            tap_b = %tap_b,
            "P2p link registered — eBPF redirect will attach after node creation"
        );
    }

    tracing::info!(
        lab_id = %lab_id,
        links_created = links_detailed.len(),
        "All point-to-point links created"
    );

    // Record veths for disabled interfaces on P2p containers.
    // P2p containers handle ALL data interfaces via veth+netns (not Docker networks)
    // to avoid ethN naming conflicts between Docker and the container's init process.
    for nsd in node_setup_data {
        if !p2p_container_names.contains(&nsd.name) {
            continue;
        }
        let node_data = lab_node_data
            .iter()
            .find(|n| n.name == nsd.name)
            .ok_or_else(|| anyhow!("P2p container {} not found in lab_node_data", nsd.name))?;

        for iface in &nsd.interfaces {
            if !matches!(iface.data, data::NodeInterface::Disabled) {
                continue;
            }
            // A disabled interface's veth stays down, with no eBPF.
            // Format: cd{node_idx}i{iface_idx}-{lab_id}, compact to stay within Linux's
            // 15-char interface name limit.
            p2p_container_veths.push(P2pContainerVeth {
                node_name: nsd.name.clone(),
                host_veth: format!(
                    "{CONTAINER_HOST_VETH_PREFIX}{}i{}-{lab_id}",
                    nsd.index, iface.index
                ),
                host_alias: format!("{}-p2p-disabled-host-{}::{}", lab_id, nsd.name, iface.name),
                interface_idx: iface.index,
                node_model: node_data.model,
                admin_down: true,
                bridge: None,
            });
        }
    }

    Ok(LinkRecords {
        lab_link_data,
        p2p_container_veths,
        p2p_container_names,
    })
}

/// The P2p link ends that exist, by name, and the links whose eBPF redirect is attached. Links
/// with a container end are attached as their containers start.
pub(super) struct P2pAttachment {
    pub(super) ready_ends: HashSet<String>,
    pub(super) attached: HashSet<u16>,
}

/// Attach the eBPF redirect on every P2p link whose ends are both VM or unikernel taps, which
/// exist once libvirt has created the domains (phase 11b).
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn attach_vm_links(
    build: &LabBuild<'_>,
    p2p_links: &[&data::LabLinkData],
    p2p_container_veths: &[P2pContainerVeth],
    links_detailed: &[topology::LinkDetailed],
) -> Result<P2pAttachment> {
    let container_host_veths: HashSet<&str> = p2p_container_veths
        .iter()
        .map(|veth| veth.host_veth.as_str())
        .collect();
    let ready_ends = p2p_links
        .iter()
        .flat_map(|link| [&link.tap_a, &link.tap_b])
        .filter(|tap| !container_host_veths.contains(tap.as_str()))
        .cloned()
        .collect();
    let mut attachment = P2pAttachment {
        ready_ends,
        attached: HashSet::new(),
    };
    if !p2p_links.is_empty() {
        let attached = attach_ready_p2p_links(
            build.lab_id,
            p2p_links,
            links_detailed,
            &attachment.ready_ends,
            &mut attachment.attached,
        )
        .await?;
        let _ = build.progress.send_status(
            format!("Attached eBPF redirect on {attached} P2p links"),
            StatusKind::Done,
        );
    }
    Ok(attachment)
}
