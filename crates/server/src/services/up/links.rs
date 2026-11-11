//! Point-to-point links and the P2p container veths, and attaching their eBPF redirects.

use std::collections::HashSet;

use anyhow::{Context, Result, anyhow};
use tracing::instrument;

use super::context::LabBuild;
use crate::services::p2p_links::{ContainerVeth, KernelHost, LinkHost, LinkWiring, attach_link};

use shared::data;
use shared::data::{LinkImpairment, RecordId, StatusKind};
use shared::konst::{CONTAINER_HOST_VETH_PREFIX, TAP_PREFIX};

/// A data interface of the P2p container `node_name`, made once the container has started.
pub(super) struct P2pContainerVeth {
    pub(super) node_name: String,
    pub(super) veth: ContainerVeth,
}

/// Attach the eBPF redirect, and any impairment, on each P2p link not yet in `attached` whose
/// ends both exist, by name in `ready`. Returns how many links it attached.
#[instrument(skip_all, fields(%lab_id), level = "debug")]
pub(super) async fn attach_ready_p2p_links(
    host: &impl LinkHost,
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

        // The manifest's impairment, if any, applies to both directions.
        let impairment = links_detailed
            .iter()
            .find(|l| l.link_idx == link_data.index)
            .and_then(|l| l.impairment.as_ref())
            .map(LinkImpairment::try_from)
            .transpose()?
            .unwrap_or_default();
        attach_link(
            host,
            &LinkWiring {
                tap_a,
                tap_b,
                ifindex_a,
                ifindex_b,
                a_to_b: &impairment,
                b_to_a: &impairment,
            },
        )
        .await?;

        tracing::info!(lab_id = %lab_id, %tap_a, %tap_b, "Attached P2p link");
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
            }));
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
            }));
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
    for bridge in bridge_connections {
        for link in &bridge.links {
            if link.node_name == node_name && link.interface_name == *interface_name {
                interface_data = Some(data::NodeInterface::Bridge(data::BridgeInterface {
                    name: bridge.bridge_name.clone(),
                }));
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
    let lab_id = build.lab_id;

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
        let node_a = lab_node(lab_node_data, &link.node_a)?;
        let node_b = lab_node(lab_node_data, &link.node_b)?;
        let link_data = record_link(build, lab_record_id, idx, link, node_a, node_b).await?;

        // P2p link: for VMs, libvirt creates the tap devices via type='ethernet'.
        // A container end is a veth created into the container's netns once it has
        // started, its host end named as the tap. eBPF redirect is attached once both
        // ends exist.
        let ends = [
            (
                node_a,
                &link.node_a,
                &link.int_a,
                link.int_a_idx,
                link.node_a_model,
                &link_data.tap_a,
            ),
            (
                node_b,
                &link.node_b,
                &link.int_b,
                link.int_b_idx,
                link.node_b_model,
                &link_data.tap_b,
            ),
        ];
        for (node, node_name, interface, interface_idx, node_model, tap) in ends {
            if node.kind != data::NodeKind::Container {
                continue;
            }
            p2p_container_veths.push(P2pContainerVeth {
                node_name: node_name.clone(),
                veth: ContainerVeth {
                    host_veth: tap.clone(),
                    host_alias: format!("{lab_id}-p2p-host-{node_name}::{interface}"),
                    interface_idx,
                    node_model,
                    admin_down: false,
                    bridge: None,
                },
            });
            p2p_container_names.insert(node_name.clone());
        }

        tracing::info!(
            lab_id = %lab_id,
            tap_a = %link_data.tap_a,
            tap_b = %link_data.tap_b,
            "P2p link registered — eBPF redirect will attach after node creation"
        );
        lab_link_data.push(link_data);
    }

    tracing::info!(
        lab_id = %lab_id,
        links_created = links_detailed.len(),
        "All point-to-point links created"
    );

    p2p_container_veths.extend(disabled_container_veths(
        lab_id,
        node_setup_data,
        lab_node_data,
        &p2p_container_names,
    )?);

    Ok(LinkRecords {
        lab_link_data,
        p2p_container_veths,
        p2p_container_names,
    })
}

/// The lab node called `name`.
fn lab_node<'a>(
    lab_node_data: &'a [data::LabNodeData],
    name: &str,
) -> Result<&'a data::LabNodeData> {
    lab_node_data
        .iter()
        .find(|n| n.name == name)
        .ok_or_else(|| anyhow!("Node not found: {name}"))
}

/// Record one P2p link, numbered `idx` in the manifest, with its taps and the manifest's
/// impairment in both directions.
#[instrument(skip_all, fields(lab_id = %build.lab_id, link = idx), level = "debug")]
async fn record_link(
    build: &LabBuild<'_>,
    lab_record_id: &RecordId,
    idx: usize,
    link: &topology::LinkDetailed,
    node_a: &data::LabNodeData,
    node_b: &data::LabNodeData,
) -> Result<data::LabLinkData> {
    let lab_id = build.lab_id;
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
        build.db,
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

    let _ = build.progress.send_status(
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

    Ok(data::LabLinkData {
        index: link.link_idx,
        kind: link_kind,
        node_a: node_a.record.clone(),
        node_b: node_b.record.clone(),
        int_a: link.int_a.clone(),
        int_b: link.int_b.clone(),
        tap_a,
        tap_b,
    })
}

/// A veth for each disabled interface of the containers on a P2p link. Those containers take
/// every data interface as a veth in their netns, not a Docker network, so Docker and the
/// container's init cannot both name `ethN`. A disabled interface's veth stays down, with no
/// eBPF.
fn disabled_container_veths(
    lab_id: &str,
    node_setup_data: &[data::NodeSetupData],
    lab_node_data: &[data::LabNodeData],
    p2p_container_names: &HashSet<String>,
) -> Result<Vec<P2pContainerVeth>> {
    let mut veths = vec![];
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
            // Format: cd{node_idx}i{iface_idx}-{lab_id}, compact to stay within Linux's
            // 15-char interface name limit.
            veths.push(P2pContainerVeth {
                node_name: nsd.name.clone(),
                veth: ContainerVeth {
                    host_veth: format!(
                        "{CONTAINER_HOST_VETH_PREFIX}{}i{}-{lab_id}",
                        nsd.index, iface.index
                    ),
                    host_alias: format!(
                        "{}-p2p-disabled-host-{}::{}",
                        lab_id, nsd.name, iface.name
                    ),
                    interface_idx: iface.index,
                    node_model: node_data.model,
                    admin_down: true,
                    bridge: None,
                },
            });
        }
    }
    Ok(veths)
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
        .map(|veth| veth.veth.host_veth.as_str())
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
            &KernelHost::new(build.docker_conn),
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
