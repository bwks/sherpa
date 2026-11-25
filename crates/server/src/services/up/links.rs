//! Point-to-point links, container veths, and attaching the links' eBPF redirects.

use std::collections::{BTreeSet, HashSet};

use anyhow::{Context, Result, anyhow, bail};
use tracing::instrument;

use super::context::LabBuild;
use crate::konst::{CONTAINER_HOST_VETH_PREFIX, TAP_PREFIX};
use crate::services::error::ServiceError;
use crate::services::p2p_links::{ContainerVeth, KernelHost, LinkHost, LinkWiring, attach_link};

use shared::data;
use shared::data::{LinkImpairment, RecordId, StatusKind};

/// A data interface of container `node_name`, made once the container has started.
pub(super) struct P2pContainerVeth {
    pub(super) node_name: String,
    pub(super) veth: ContainerVeth,
}

pub(super) struct ReadyLinksAttached {
    pub(super) count: usize,
    pub(super) gone_ends: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct GoneEnd<'a> {
    pub(super) end: &'a str,
    pub(super) node: &'a str,
}

/// Attach the eBPF redirect, and any impairment, on each P2p link not yet in `attached` whose
/// ends both exist, by name in `ready`. Returns the count attached and the ends gone.
#[instrument(skip_all, fields(%lab_id), level = "debug")]
pub(super) async fn attach_ready_p2p_links(
    host: &impl LinkHost,
    lab_id: &str,
    p2p_links: &[&data::LabLinkData],
    links_detailed: &[topology::LinkDetailed],
    ready: &HashSet<String>,
    attached: &mut HashSet<u16>,
) -> Result<ReadyLinksAttached> {
    let mut outcome = ReadyLinksAttached {
        count: 0,
        gone_ends: vec![],
    };
    for link_data in p2p_links {
        if attached.contains(&link_data.index)
            || !ready.contains(&link_data.tap_a)
            || !ready.contains(&link_data.tap_b)
        {
            continue;
        }
        let tap_a = &link_data.tap_a;
        let tap_b = &link_data.tap_b;

        let looked_up = [
            (tap_a, look_up_end(host, tap_a).await?),
            (tap_b, look_up_end(host, tap_b).await?),
        ];
        let [(_, Some(ifindex_a)), (_, Some(ifindex_b))] = looked_up else {
            let gone: Vec<String> = looked_up
                .into_iter()
                .filter(|(_, ifindex)| ifindex.is_none())
                .map(|(end, _)| end.clone())
                .collect();
            tracing::warn!(lab_id = %lab_id, ?gone, "P2p link end gone before its link was attached");
            outcome.gone_ends.extend(gone);
            continue;
        };

        // The manifest's impairment, if any, applies to both directions.
        let impairment = links_detailed
            .iter()
            .find(|l| l.link_idx == link_data.index)
            .and_then(|l| l.impairment.as_ref())
            .map(LinkImpairment::try_from)
            .transpose()?
            .unwrap_or_default();
        let wiring = LinkWiring {
            tap_a,
            tap_b,
            ifindex_a,
            ifindex_b,
            a_to_b: &impairment,
            b_to_a: &impairment,
        };
        if let Err(error) = attach_link(host, &wiring).await {
            let mut gone = vec![];
            for end in [tap_a, tap_b] {
                if look_up_end(host, end).await?.is_none() {
                    gone.push(end.clone());
                }
            }
            if gone.is_empty() {
                return Err(error);
            }
            tracing::warn!(lab_id = %lab_id, ?gone, error = %format!("{error:#}"), "P2p link end gone while its link was attached");
            outcome.gone_ends.extend(gone);
            continue;
        }

        tracing::info!(lab_id = %lab_id, %tap_a, %tap_b, "Attached P2p link");
        attached.insert(link_data.index);
        outcome.count += 1;
    }
    Ok(outcome)
}

/// The interface index of link end `end`, or `None` when it is not on the host.
async fn look_up_end(host: &impl LinkHost, end: &str) -> Result<Option<u32>> {
    host.ifindex(end)
        .await
        .with_context(|| format!("failed to look up P2p link end {end}"))
}

/// The container node that owns each P2p link end in `gone`.
pub(super) fn gone_end_owners<'a>(
    gone: &'a [String],
    veths: &'a [P2pContainerVeth],
) -> Result<Vec<GoneEnd<'a>>> {
    gone.iter()
        .map(|end| {
            veths
                .iter()
                .find(|veth| veth.veth.host_veth == *end)
                .map(|veth| GoneEnd {
                    end,
                    node: &veth.node_name,
                })
                .ok_or_else(|| anyhow!("P2p link end {end} is gone from the host"))
        })
        .collect()
}

/// The nodes of `owners` whose containers are not `running`.
pub(super) fn exited_owners<'a>(
    owners: &[GoneEnd<'a>],
    running: &HashSet<&str>,
) -> Result<BTreeSet<&'a str>> {
    let mut exited = BTreeSet::new();
    for owner in owners {
        if running.contains(owner.node) {
            bail!(ServiceError::LinkEndGone {
                end: owner.end.to_owned(),
                node: owner.node.to_owned(),
            });
        }
        exited.insert(owner.node);
    }
    Ok(exited)
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
    pub(super) veth_container_names: HashSet<String>,
}

/// Record every P2p link, and plan the link end and disabled interface veths of containers.
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn create_links(
    build: &LabBuild<'_>,
    lab_record_id: &RecordId,
    links_detailed: &[topology::LinkDetailed],
    bridges_detailed: &[topology::BridgeDetailed],
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
    let veth_container_names = veth_container_names(links_detailed, bridges_detailed);

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
                link.node_a_model_kind,
                &link_data.tap_a,
            ),
            (
                node_b,
                &link.node_b,
                &link.int_b,
                link.int_b_idx,
                link.node_b_model_kind,
                &link_data.tap_b,
            ),
        ];
        for (node, node_name, interface, interface_idx, model_kind, tap) in ends {
            if node.model_kind.kind() != data::NodeKind::Container {
                continue;
            }
            p2p_container_veths.push(P2pContainerVeth {
                node_name: node_name.clone(),
                veth: ContainerVeth {
                    host_veth: tap.clone(),
                    host_alias: format!("{lab_id}-p2p-host-{node_name}::{interface}"),
                    interface_idx,
                    model_kind,
                    admin_down: false,
                    bridge: None,
                },
            });
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
        &veth_container_names,
    )?);

    Ok(LinkRecords {
        lab_link_data,
        p2p_container_veths,
        veth_container_names,
    })
}

/// The containers that take every data interface as a veth: those on a link or bridge.
fn veth_container_names(
    links_detailed: &[topology::LinkDetailed],
    bridges_detailed: &[topology::BridgeDetailed],
) -> HashSet<String> {
    let link_ends = links_detailed.iter().flat_map(|link| {
        [
            (&link.node_a, link.node_a_model_kind),
            (&link.node_b, link.node_b_model_kind),
        ]
    });
    let bridge_members = bridges_detailed.iter().flat_map(|bridge| {
        bridge
            .links
            .iter()
            .map(|member| (&member.node_name, member.node_model_kind))
    });
    link_ends
        .chain(bridge_members)
        .filter(|(_, model_kind)| model_kind.kind() == data::NodeKind::Container)
        .map(|(name, _)| name.clone())
        .collect()
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

/// A veth, kept down, for each disabled interface of the containers on a link or bridge.
fn disabled_container_veths(
    lab_id: &str,
    node_setup_data: &[data::NodeSetupData],
    lab_node_data: &[data::LabNodeData],
    veth_container_names: &HashSet<String>,
) -> Result<Vec<P2pContainerVeth>> {
    let mut veths = vec![];
    for nsd in node_setup_data {
        if !veth_container_names.contains(&nsd.name) {
            continue;
        }
        let node_data = lab_node_data
            .iter()
            .find(|n| n.name == nsd.name)
            .ok_or_else(|| anyhow!("Container {} not found in lab_node_data", nsd.name))?;

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
                    model_kind: node_data.model_kind,
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
            &KernelHost::new(build.docker_conn, build.qemu),
            build.lab_id,
            p2p_links,
            links_detailed,
            &attachment.ready_ends,
            &mut attachment.attached,
        )
        .await?;
        if !attached.gone_ends.is_empty() {
            bail!(
                "P2p link ends gone from the host after their nodes were created: {}",
                attached.gone_ends.join(", ")
            );
        }
        let _ = build.progress.send_status(
            format!("Attached eBPF redirect on {} P2p links", attached.count),
            StatusKind::Done,
        );
    }
    Ok(attachment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::p2p_links::fake::FakeHost;

    use shared::data::{BridgeKind, DbNode, ModelKind, NodeModel, NodeState, ReadyCheck};

    const LAB_ID: &str = "lab";

    fn db_node(name: &str) -> DbNode {
        DbNode {
            id: None,
            name: name.to_string(),
            image: RecordId::new("node_image", "x"),
            index: 1,
            lab: RecordId::new("lab", LAB_ID),
            mgmt_ipv4: None,
            mgmt_ipv6: None,
            mgmt_mac: None,
            state: NodeState::Building,
            cpu_count: None,
            memory: None,
            ssh_host_key: None,
            ready_check: ReadyCheck::default(),
            ssh_username: None,
        }
    }

    fn link(index: u16, node_a: &str, node_b: &str) -> data::LabLinkData {
        data::LabLinkData {
            index,
            kind: BridgeKind::P2p,
            node_a: db_node(node_a),
            node_b: db_node(node_b),
            int_a: "eth1".to_string(),
            int_b: "eth1".to_string(),
            tap_a: format!("tpa{index}"),
            tap_b: format!("tpb{index}"),
        }
    }

    fn veth(node_name: &str, host_veth: &str) -> P2pContainerVeth {
        P2pContainerVeth {
            node_name: node_name.to_string(),
            veth: ContainerVeth {
                host_veth: host_veth.to_string(),
                host_alias: format!("alias-{host_veth}"),
                interface_idx: 1,
                model_kind: ModelKind::default_for(NodeModel::FrrLinux),
                admin_down: false,
                bridge: None,
            },
        }
    }

    fn detailed_link(node_a: &str, node_b: &str) -> topology::LinkDetailed {
        topology::LinkDetailed {
            node_a: node_a.to_string(),
            node_a_idx: 1,
            node_a_model_kind: ModelKind::default_for(NodeModel::FrrLinux),
            int_a: "eth1".to_string(),
            int_a_idx: 1,
            node_b: node_b.to_string(),
            node_b_idx: 2,
            node_b_model_kind: ModelKind::default_for(NodeModel::FrrLinux),
            int_b: "eth1".to_string(),
            int_b_idx: 1,
            link_idx: 0,
            impairment: None,
        }
    }

    fn member(node_name: &str, model_kind: ModelKind) -> topology::BridgeLinkDetailed {
        topology::BridgeLinkDetailed {
            node_name: node_name.to_string(),
            node_model_kind: model_kind,
            interface_name: "eth2".to_string(),
            interface_index: 2,
        }
    }

    #[test]
    fn private_bridge_013_a_container_member_on_no_link_takes_its_data_interfaces_as_veths() {
        let bridge = topology::BridgeDetailed {
            manifest_name: "private".to_string(),
            bridge_name: format!("brs0-{LAB_ID}"),
            libvirt_name: format!("sherpa-bridge0-private-{LAB_ID}"),
            index: 0,
            links: vec![
                member("c1", ModelKind::default_for(NodeModel::FrrLinux)),
                member("r1", ModelKind::default_for(NodeModel::CiscoIol)),
                member("linux01", ModelKind::default_for(NodeModel::AlpineLinux)),
            ],
        };
        let names = veth_container_names(&[detailed_link("c1", "c2")], &[bridge]);
        assert_eq!(
            names,
            HashSet::from(["c1", "c2", "r1"].map(String::from)),
            "linux01 is a VM, so libvirt joins it"
        );
        assert!(veth_container_names(&[], &[]).is_empty());
    }

    fn three_containers() -> (FakeHost, Vec<data::LabLinkData>, Vec<P2pContainerVeth>) {
        let host = FakeHost::default();
        host.make(&["tpa0", "tpb0", "tpa1", "tpb1"]);
        let links = vec![link(0, "c1", "c2"), link(1, "c1", "c3")];
        let veths = vec![
            veth("c1", "tpa0"),
            veth("c2", "tpb0"),
            veth("c1", "tpa1"),
            veth("c3", "tpb1"),
        ];
        (host, links, veths)
    }

    fn ready(ends: &[&str]) -> HashSet<String> {
        ends.iter().map(ToString::to_string).collect()
    }

    #[tokio::test]
    async fn p2p_link_017_a_container_whose_end_is_gone_at_its_lookup_fails_as_a_node_and_up_carries_on()
     {
        let (host, links, veths) = three_containers();
        host.remove(&["tpa0", "tpa1"]);
        host.make(&["tpa2", "tpb2"]);
        let links: Vec<data::LabLinkData> =
            links.into_iter().chain([link(2, "c2", "c3")]).collect();
        let link_refs: Vec<&data::LabLinkData> = links.iter().collect();
        let mut attached = HashSet::new();

        let outcome = attach_ready_p2p_links(
            &host,
            LAB_ID,
            &link_refs,
            &[],
            &ready(&["tpa0", "tpb0", "tpa1", "tpb1", "tpa2", "tpb2"]),
            &mut attached,
        )
        .await
        .expect("a gone end does not fail the attach");

        assert_eq!(outcome.gone_ends, vec!["tpa0", "tpa1"]);
        assert_eq!(
            attached,
            HashSet::from([2]),
            "the link between c2 and c3 is attached"
        );
        assert!(host.wired("tpa2", "tpb2"));
        let owners = gone_end_owners(&outcome.gone_ends, &veths).unwrap();
        let exited = exited_owners(&owners, &HashSet::from(["c2", "c3"])).unwrap();
        assert_eq!(exited, BTreeSet::from(["c1"]));
    }

    #[tokio::test]
    async fn p2p_link_018_a_container_whose_end_goes_while_its_link_is_attached_fails_as_a_node_and_up_carries_on()
     {
        let (mut host, links, veths) = three_containers();
        host.vanishing_redirects.insert("tpa0".to_string());
        let link_refs: Vec<&data::LabLinkData> = links.iter().take(1).collect();
        let mut attached = HashSet::new();

        let outcome = attach_ready_p2p_links(
            &host,
            LAB_ID,
            &link_refs,
            &[],
            &ready(&["tpa0", "tpb0"]),
            &mut attached,
        )
        .await
        .expect("an end gone at the attach does not fail the attach");

        assert_eq!(outcome.gone_ends, vec!["tpa0"]);
        assert!(attached.is_empty(), "the link is left unattached");
        let owners = gone_end_owners(&outcome.gone_ends, &veths).unwrap();
        let exited = exited_owners(&owners, &HashSet::from(["c2"])).unwrap();
        assert_eq!(exited, BTreeSet::from(["c1"]));
    }

    #[tokio::test]
    async fn p2p_link_019_an_end_gone_while_its_container_runs_fails_up_carrying_the_end_and_its_node()
     {
        let (host, links, veths) = three_containers();
        host.remove(&["tpa0"]);
        let link_refs: Vec<&data::LabLinkData> = links.iter().take(1).collect();
        let mut attached = HashSet::new();
        let outcome = attach_ready_p2p_links(
            &host,
            LAB_ID,
            &link_refs,
            &[],
            &ready(&["tpa0", "tpb0"]),
            &mut attached,
        )
        .await
        .unwrap();
        let owners = gone_end_owners(&outcome.gone_ends, &veths).unwrap();

        let error = exited_owners(&owners, &HashSet::from(["c1", "c2"]))
            .expect_err("an end gone from a running container fails up");

        assert_eq!(
            error.downcast_ref::<ServiceError>(),
            Some(&ServiceError::LinkEndGone {
                end: "tpa0".to_owned(),
                node: "c1".to_owned(),
            })
        );
    }
}
