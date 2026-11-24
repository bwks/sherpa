//! Repairing a lab's P2p links after their ends were made again outside Sherpa: a VM started
//! with `virsh`, a container restarted with `docker`, either after a host reboot.
//!
//! The check needs nothing remembered from an earlier scan. The kernel shows what is wrong: an
//! end of a running node that does not exist (a restarted container's veths go with its old
//! network namespace), or one that exists with no redirect filter (libvirt made a new, bare
//! tap; a filter cannot move between interfaces). So the first scan after sherpad starts
//! repairs whatever changed while it was down.
//!
//! Only running nodes are checked and repaired: a stopped or paused node is left as it is, and
//! its links wait for it. An end with no filter is only repaired when its peer's end exists, as
//! nothing can be attached before that.
//!
//! The check also gives the ends it found made again, missing or with no filter, so their
//! carrier is set whatever the link's state was (see `carrier`).

use std::collections::{HashMap, HashSet};
use std::fmt;

use tracing::instrument;

use super::host::LinkHost;
use super::reattach::{LabLinks, LinkOutcome, RecreatedEnd, reattach_node_links};
use super::veths::{BridgeEnd, ContainerVeth};
use crate::daemon::unreadable_ends::{EndRead, UnreadableEnd, UnreadableEnds};
use crate::services::error::Unexpected;

use shared::data::{DbLink, LinkState, RecordId};

/// What is wrong with an end of a running node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EndFault {
    Missing,
    NoRedirect,
}

/// One check of a lab's links: the ends to repair, the ends made again, and the state of each
/// link as the kernel shows it, which a repair replaces with what it left the link in.
pub(crate) struct LinkCheck {
    broken: Vec<RecordId>,
    fresh: HashSet<String>,
    states: HashMap<u16, LinkState>,
}

/// What the kernel shows of one end of a running node.
enum Seen {
    Present { redirect: bool },
    Absent,
    Unreadable(EndRead, anyhow::Error),
}

/// What the kernel shows of a running container's private bridge member.
enum Member {
    Veth,
    Missing,
    Older,
    Unreadable(anyhow::Error),
}

impl fmt::Display for EndFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EndFault::Missing => write!(f, "does not exist"),
            EndFault::NoRedirect => write!(f, "has no redirect"),
        }
    }
}

impl LinkCheck {
    /// The nodes with an end to repair, each once, in the order their ends were found.
    pub(crate) fn broken_nodes(&self) -> Vec<RecordId> {
        self.broken.clone()
    }

    /// The host interfaces of the ends made again: missing, or with no filter.
    pub(crate) fn fresh_ends(&self) -> HashSet<String> {
        self.fresh.clone()
    }

    /// Take what repairing a node left its links in.
    pub(crate) fn record(&mut self, outcomes: &[LinkOutcome]) {
        for outcome in outcomes {
            self.states
                .insert(outcome.index, outcome.report.state.clone());
        }
    }

    /// Each link's state, by its index.
    pub(crate) fn into_states(self) -> HashMap<u16, LinkState> {
        self.states
    }
}

/// Check the lab's links and private bridge member ends of the `running` nodes in the kernel.
#[instrument(skip_all, level = "debug")]
pub(crate) async fn check_lab_links(
    host: &impl LinkHost,
    lab: &LabLinks,
    bridge_ends: &[BridgeEnd],
    running: &HashSet<RecordId>,
    unreadable: &UnreadableEnds,
) -> LinkCheck {
    let mut links: Vec<&DbLink> = lab.links().iter().collect();
    links.sort_by_key(|link| link.index);
    let mut check = LinkCheck {
        broken: vec![],
        fresh: HashSet::new(),
        states: HashMap::new(),
    };
    for link in links {
        let state = check_link(host, lab, link, running, unreadable, &mut check).await;
        check.states.insert(link.index, state);
    }
    for end in bridge_ends {
        if running.contains(&end.node) {
            check_bridge_end(host, lab, end, unreadable, &mut check).await;
        }
    }
    check
}

/// Check one private bridge member end of a running container.
async fn check_bridge_end(
    host: &impl LinkHost,
    lab: &LabLinks,
    end: &BridgeEnd,
    unreadable: &UnreadableEnds,
    check: &mut LinkCheck,
) {
    let member = see_member(host, end).await;
    let failure = match &member {
        Member::Unreadable(error) => Some((EndRead::LookUp, error)),
        Member::Veth | Member::Missing | Member::Older => None,
    };
    let _ = record_end_read(unreadable, lab.lab_id(), &end.host_veth, failure);
    match member {
        Member::Missing => {
            tracing::info!(node = %lab.name(&end.node), iface = %end.host_veth, fault = %EndFault::Missing, "Private bridge member end needs repair");
            if !check.broken.contains(&end.node) {
                check.broken.push(end.node.clone());
            }
        }
        Member::Older => {
            if unreadable.note_older_member(lab.lab_id()) {
                tracing::info!(lab_id = %lab.lab_id(), node = %lab.name(&end.node), iface = %end.interface, "Private bridge member is from an older release; redeploy the node to make it a veth");
            }
        }
        Member::Veth | Member::Unreadable(_) => {}
    }
}

/// What the kernel shows of private bridge member `end`.
async fn see_member(host: &impl LinkHost, end: &BridgeEnd) -> Member {
    match host.ifindex(&end.host_veth).await {
        Ok(Some(_)) => return Member::Veth,
        Ok(None) => {}
        Err(error) => return Member::Unreadable(error),
    }
    match host
        .container_has_interface(&end.container, &end.interface)
        .await
    {
        Ok(true) => Member::Older,
        Ok(false) => Member::Missing,
        Err(error) => Member::Unreadable(error),
    }
}

/// Check one link: add the nodes of its ends that need repair, and its ends made again, to
/// `check`, and return its state as the kernel shows it.
async fn check_link(
    host: &impl LinkHost,
    lab: &LabLinks,
    link: &DbLink,
    running: &HashSet<RecordId>,
    unreadable_ends: &UnreadableEnds,
    check: &mut LinkCheck,
) -> LinkState {
    let ends = [
        (&link.node_a, &link.tap_a, &link.tap_b),
        (&link.node_b, &link.tap_b, &link.tap_a),
    ];
    let mut faults = vec![];
    let mut unreadable = None;
    for (node, iface, peer_iface) in ends {
        if !running.contains(node) {
            continue;
        }
        let seen = see(host, iface).await;
        let read_failure = match &seen {
            Seen::Unreadable(read, error) => Some((*read, error)),
            Seen::Present { .. } | Seen::Absent => None,
        };
        let unreadable_reason = record_end_read(unreadable_ends, lab.lab_id(), iface, read_failure);
        if matches!(seen, Seen::Absent | Seen::Present { redirect: false }) {
            check.fresh.insert(iface.clone());
        }
        let fault = match seen {
            Seen::Absent => Some(EndFault::Missing),
            Seen::Present { redirect: false } if exists(host, peer_iface).await => {
                Some(EndFault::NoRedirect)
            }
            Seen::Present { .. } => None,
            Seen::Unreadable(..) => {
                unreadable = unreadable_reason;
                None
            }
        };
        if let Some(fault) = fault {
            tracing::info!(node = %lab.name(node), link = %lab.label(link), %iface, %fault, "P2p link end needs repair");
            faults.push((iface, fault));
            if !check.broken.contains(node) {
                check.broken.push(node.clone());
            }
        }
    }

    if let Some((iface, fault)) = faults.first() {
        return LinkState::Failed {
            reason: format!("its end {iface} {fault}"),
        };
    }
    if let Some(reason) = unreadable {
        return LinkState::Failed { reason };
    }
    match [&link.node_a, &link.node_b]
        .into_iter()
        .find(|node| !running.contains(*node))
    {
        Some(down) => LinkState::PeerDown {
            peer: lab.name(down),
        },
        None => LinkState::Attached,
    }
}

/// What the kernel shows of end `iface`.
async fn see(host: &impl LinkHost, iface: &str) -> Seen {
    let ifindex = match host.ifindex(iface).await {
        Ok(Some(ifindex)) => ifindex,
        Ok(None) => return Seen::Absent,
        Err(error) => return Seen::Unreadable(EndRead::LookUp, error),
    };
    match host.has_redirect(ifindex).await {
        Ok(redirect) => Seen::Present { redirect },
        Err(error) => Seen::Unreadable(EndRead::Filter, error),
    }
}

/// Keep in `ends` whether end `iface` of lab `lab_id` could be read, and log when that
/// changes: at warn level with a new reference when the end becomes unreadable or what failed
/// changes, and at info level when it can be read again. An end that stays as it was is not
/// logged again. Returns the reason the end's link fails with, while it is unreadable.
fn record_end_read(
    ends: &UnreadableEnds,
    lab_id: &str,
    iface: &str,
    failure: Option<(EndRead, &anyhow::Error)>,
) -> Option<String> {
    let Some((read, error)) = failure else {
        if ends.record(lab_id, iface, None).is_some() {
            tracing::info!(%lab_id, %iface, "P2p link end can be read again");
        }
        return None;
    };
    let unreadable = match ends.of_end(lab_id, iface) {
        Some(previous) if previous.read == read => previous,
        _ => UnreadableEnd {
            read,
            reference: Unexpected::warn(error, &format!("Failed to {read} P2p link end {iface}")),
        },
    };
    let reason = format!("failed to {read} {iface}: {}", unreadable.reference);
    let _ = ends.record(lab_id, iface, Some(unreadable));
    Some(reason)
}

/// Whether end `iface` exists. One that cannot be looked up is taken as not existing.
async fn exists(host: &impl LinkHost, iface: &str) -> bool {
    matches!(host.ifindex(iface).await, Ok(Some(_)))
}

/// Repair the links of `node`, whose ends are made as `end` says, and log what each link was
/// left in. Of a container's veths, only those that do not exist are made: making a veth
/// deletes its host end first, which would take a working interface down.
#[instrument(skip(host, lab, end), fields(%node), level = "debug")]
pub(crate) async fn repair_node(
    host: &impl LinkHost,
    lab: &LabLinks,
    node: &RecordId,
    end: RecreatedEnd,
) -> Vec<LinkOutcome> {
    let end = match end {
        RecreatedEnd::Container { container, veths } => RecreatedEnd::Container {
            container,
            veths: missing_veths(host, veths).await,
        },
        end => end,
    };
    let name = lab.name(node);
    let outcomes = reattach_node_links(host, lab, node, end).await;
    for LinkOutcome { report, .. } in &outcomes {
        match &report.state {
            LinkState::Attached => {
                tracing::info!(node = %name, link = %report.link, "Repaired P2p link");
            }
            LinkState::PeerDown { peer } => {
                tracing::info!(node = %name, link = %report.link, %peer, "P2p link waits for its peer");
            }
            LinkState::Failed { reason } => {
                tracing::warn!(node = %name, link = %report.link, error = %reason, "Failed to repair P2p link");
            }
        }
    }
    outcomes
}

/// The veths of `veths` whose host end does not exist. One that cannot be looked up is left
/// out: its link fails when it is looked up again.
async fn missing_veths(host: &impl LinkHost, veths: Vec<ContainerVeth>) -> Vec<ContainerVeth> {
    let mut missing = vec![];
    for veth in veths {
        match host.ifindex(&veth.host_veth).await {
            Ok(None) => missing.push(veth),
            Ok(Some(_)) => {}
            Err(error) => {
                tracing::warn!(host_veth = %veth.host_veth, error = %error, "Failed to look up a container veth");
            }
        }
    }
    missing
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::{ModelKind, ReadyCheck};

    use std::env;
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use bollard::Docker;
    use bollard::query_parameters::{
        CreateContainerOptions, RemoveContainerOptions, RestartContainerOptions,
        StartContainerOptions, StopContainerOptions,
    };
    use bollard::secret::{ContainerCreateBody, HostConfig};
    use container::exec_container;
    use libvirt::Qemu;
    use virt::connect::Connect;
    use virt::domain::Domain;

    use super::super::carrier::follow_carrier;
    use super::super::fake::FakeHost;
    use super::super::host::KernelHost;
    use super::super::started::follow_started_links;
    use super::super::veths::ContainerVeth;
    use crate::daemon::link_states::LinkStates;
    use crate::daemon::unreadable_ends::UnreadableEnds;
    use crate::log_capture::capture_log;
    use crate::services::node_lifecycle::query_runtimes;
    use shared::data::{BridgeKind, DbNode, LinkImpairment, NodeKind, NodeModel, NodeState};
    use shared::konst::{SHERPA_IMAGES_PATH, SHERPA_STORAGE_POOL_PATH};

    fn node(name: &str) -> DbNode {
        DbNode {
            id: Some(RecordId::new("node", name)),
            name: name.to_string(),
            image: RecordId::new("node_image", "img"),
            index: 1,
            lab: RecordId::new("lab", "lab"),
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

    fn link(index: u16, node_a: &str, node_b: &str) -> DbLink {
        DbLink {
            id: None,
            index,
            kind: BridgeKind::P2p,
            node_a: RecordId::new("node", node_a),
            node_b: RecordId::new("node", node_b),
            int_a: format!("eth{}", index + 1),
            int_b: "eth1".to_string(),
            lab: RecordId::new("lab", "lab"),
            tap_a: format!("tpa{index}"),
            tap_b: format!("tpb{index}"),
            impairment_a_to_b: LinkImpairment::default(),
            impairment_b_to_a: LinkImpairment::default(),
        }
    }

    fn id(name: &str) -> RecordId {
        RecordId::new("node", name)
    }

    fn running(names: &[&str]) -> HashSet<RecordId> {
        names.iter().map(|name| id(name)).collect()
    }

    /// vm1 linked to vm2 (0) and to ctr1 (1), every end made and wired, as `up` leaves it.
    fn wired_lab() -> (FakeHost, LabLinks) {
        let nodes = ["vm1", "vm2", "ctr1"].map(node);
        let lab = LabLinks::new(
            "lab",
            vec![link(0, "vm1", "vm2"), link(1, "vm1", "ctr1")],
            &nodes,
            HashMap::from([(id("ctr1"), NodeKind::Container)]),
        );
        let host = FakeHost::default();
        host.make(&["tpa0", "tpb0", "tpa1", "tpb1", "cd1i2-lab"]);
        for (a, b) in [("tpa0", "tpb0"), ("tpa1", "tpb1")] {
            let a_index = host.index(a).unwrap();
            let b_index = host.index(b).unwrap();
            host.redirects.borrow_mut().insert(a.to_string(), b_index);
            host.redirects.borrow_mut().insert(b.to_string(), a_index);
        }
        (host, lab)
    }

    /// ctr1's veths: its link's end, and its disabled second interface.
    fn ctr1_end() -> RecreatedEnd {
        let veth = |host_veth: &str, idx: u8, admin_down: bool| ContainerVeth {
            host_veth: host_veth.to_string(),
            host_alias: format!("alias-{host_veth}"),
            interface_idx: idx,
            model_kind: ModelKind::default_for(NodeModel::FrrLinux),
            admin_down,
            bridge: None,
        };
        RecreatedEnd::Container {
            container: "ctr1-lab".to_string(),
            veths: vec![veth("tpb1", 1, false), veth("cd1i2-lab", 2, true)],
        }
    }

    /// Plans a node's ends, as the scanner does from the database.
    type Planner = fn() -> RecreatedEnd;

    /// What one scan found and did.
    struct Scan {
        repaired: Vec<RecordId>,
        faults: Vec<String>,
        states: HashMap<u16, LinkState>,
    }

    /// One scan of the lab: check its links, repair each node with a broken end, as the scanner
    /// does with the ends it plans. A node not in `ends` is a VM.
    async fn scan(
        host: &FakeHost,
        lab: &LabLinks,
        running: &HashSet<RecordId>,
        ends: &[(&str, Planner)],
    ) -> Scan {
        let mut check = check_lab_links(host, lab, &[], running, &UnreadableEnds::default()).await;
        let mut found: Vec<(&u16, &LinkState)> = check.states.iter().collect();
        found.sort_by_key(|(index, _)| **index);
        let faults = found
            .into_iter()
            .filter_map(|(_, state)| match state {
                LinkState::Failed { reason } => Some(reason.clone()),
                _ => None,
            })
            .collect();
        let mut repaired = vec![];
        for node in check.broken_nodes() {
            let end = ends
                .iter()
                .find(|(name, _)| id(name) == node)
                .map_or(RecreatedEnd::Domain, |(_, end)| end());
            let outcomes = repair_node(host, lab, &node, end).await;
            check.record(&outcomes);
            repaired.push(node);
        }
        Scan {
            repaired,
            faults,
            states: check.into_states(),
        }
    }

    fn peer_down(peer: &str) -> LinkState {
        LinkState::PeerDown {
            peer: peer.to_string(),
        }
    }

    #[tokio::test]
    async fn link_reattach_020_a_scan_of_a_lab_in_which_nothing_changed_makes_nothing() {
        // A guest reboot, a pause and unpause, a sherpad restart: the ends and their filters
        // are as they were.
        let (host, lab) = wired_lab();
        let scan = scan(&host, &lab, &running(&["vm1", "vm2", "ctr1"]), &[]).await;
        assert_eq!(scan.repaired, Vec::<RecordId>::new());
        assert_eq!(host.writes.get(), 0, "no host change at all");
        assert_eq!(
            scan.states,
            HashMap::from([(0, LinkState::Attached), (1, LinkState::Attached)])
        );
    }

    #[tokio::test]
    async fn link_reattach_022_a_stopped_or_paused_node_is_not_repaired_and_its_links_wait() {
        {
            // `virsh destroy` or `shutdown`: vm2's tap goes with its filter; vm1's end stays.
            let (host, lab) = wired_lab();
            host.remove(&["tpb0"]);
            let scan = scan(&host, &lab, &running(&["vm1", "ctr1"]), &[]).await;
            assert_eq!(scan.repaired, Vec::<RecordId>::new());
            assert_eq!(host.writes.get(), 0);
            assert_eq!(scan.states[&0], peer_down("vm2"));
            assert_eq!(scan.states[&1], LinkState::Attached);
        }

        {
            // `docker stop`, an OOM kill, the main process exiting.
            let (host, lab) = wired_lab();
            host.remove(&["tpb1", "cd1i2-lab"]);
            let scan = scan(
                &host,
                &lab,
                &running(&["vm1", "vm2"]),
                &[("ctr1", ctr1_end)],
            )
            .await;
            assert_eq!(scan.repaired, Vec::<RecordId>::new());
            assert_eq!(host.writes.get(), 0);
            assert!(host.index("tpb1").is_none(), "nothing is made for it");
            assert_eq!(scan.states[&1], peer_down("ctr1"));
        }

        {
            // `virsh suspend` after the VM was started outside Sherpa: its tap is bare, but the
            // node is not running, so nothing is done to it.
            let (host, lab) = wired_lab();
            host.make(&["tpb0"]);
            let scan = scan(&host, &lab, &running(&["vm1", "ctr1"]), &[]).await;
            assert_eq!(scan.repaired, Vec::<RecordId>::new());
            assert_eq!(host.writes.get(), 0);
            assert_eq!(scan.states[&0], peer_down("vm2"));
        }
    }

    #[tokio::test]
    async fn link_reattach_030_a_vm_started_outside_sherpa_has_its_bare_taps_wired() {
        // `virsh start`: libvirt made vm2 a new tap with no filter; vm1's points at the old one.
        let (host, lab) = wired_lab();
        host.make(&["tpb0"]);
        assert!(!host.wired("tpa0", "tpb0"));
        let scan = scan(&host, &lab, &running(&["vm1", "vm2", "ctr1"]), &[]).await;
        assert_eq!(scan.faults, vec!["its end tpb0 has no redirect"]);
        assert_eq!(scan.repaired, vec![id("vm2")]);
        assert!(host.wired("tpa0", "tpb0"));
        assert_eq!(scan.states[&0], LinkState::Attached);
    }

    #[tokio::test]
    async fn link_reattach_031_a_restarted_container_gets_its_interfaces_back_and_its_links() {
        // `docker restart`, or `docker stop` then `start`: the new namespace has none of the
        // container's data interfaces.
        let (host, lab) = wired_lab();
        host.remove(&["tpb1", "cd1i2-lab"]);
        let scan = scan(
            &host,
            &lab,
            &running(&["vm1", "vm2", "ctr1"]),
            &[("ctr1", ctr1_end)],
        )
        .await;
        assert_eq!(scan.faults, vec!["its end tpb1 does not exist"]);
        assert_eq!(scan.repaired, vec![id("ctr1")]);
        assert!(host.wired("tpa1", "tpb1"));
        assert!(
            host.index("cd1i2-lab").is_some(),
            "the disabled interface is made again too"
        );
        assert_eq!(scan.states[&1], LinkState::Attached);
        assert!(host.wired("tpa0", "tpb0"), "vm1's other link is untouched");
    }

    #[tokio::test]
    async fn link_reattach_021_only_the_missing_interfaces_of_a_container_are_made() {
        // Making a veth deletes its host end first, which would flap a healthy interface.
        let (host, lab) = wired_lab();
        let disabled = host.index("cd1i2-lab");
        host.remove(&["tpb1"]);
        scan(
            &host,
            &lab,
            &running(&["vm1", "vm2", "ctr1"]),
            &[("ctr1", ctr1_end)],
        )
        .await;
        assert!(host.wired("tpa1", "tpb1"));
        assert_eq!(host.index("cd1i2-lab"), disabled);
    }

    fn bridged_ctr1_lab() -> LabLinks {
        LabLinks::new(
            "lab",
            vec![],
            &[node("ctr1")],
            HashMap::from([(id("ctr1"), NodeKind::Container)]),
        )
    }

    fn bridged_ctr1_ends() -> Vec<BridgeEnd> {
        vec![BridgeEnd {
            node: id("ctr1"),
            host_veth: "cb1i1-lab".to_string(),
            container: "ctr1-lab".to_string(),
            interface: "eth1".to_string(),
        }]
    }

    fn bridged_ctr1_end() -> RecreatedEnd {
        let veth = |host_veth: &str, idx: u8, bridge: Option<&str>| ContainerVeth {
            host_veth: host_veth.to_string(),
            host_alias: format!("alias-{host_veth}"),
            interface_idx: idx,
            model_kind: ModelKind::default_for(NodeModel::FrrLinux),
            admin_down: bridge.is_none(),
            bridge: bridge.map(str::to_string),
        };
        RecreatedEnd::Container {
            container: "ctr1-lab".to_string(),
            veths: vec![
                veth("cb1i1-lab", 1, Some("brs0-lab")),
                veth("cd1i2-lab", 2, None),
            ],
        }
    }

    #[tokio::test]
    async fn private_bridge_014_a_container_on_a_bridge_restarted_outside_sherpa_is_repaired_by_one_scan()
     {
        let lab = bridged_ctr1_lab();
        let ends = bridged_ctr1_ends();
        let host = FakeHost::default();
        let all = running(&["ctr1"]);

        let check = check_lab_links(&host, &lab, &ends, &all, &UnreadableEnds::default()).await;
        assert_eq!(check.broken_nodes(), vec![id("ctr1")]);
        repair_node(&host, &lab, &id("ctr1"), bridged_ctr1_end()).await;
        assert_eq!(
            host.bridge_ports
                .borrow()
                .get("cb1i1-lab")
                .map(String::as_str),
            Some("brs0-lab")
        );
        assert!(host.index("cd1i2-lab").is_some());

        host.writes.set(0);
        let check = check_lab_links(&host, &lab, &ends, &all, &UnreadableEnds::default()).await;
        assert_eq!(check.broken_nodes(), Vec::<RecordId>::new());
        assert_eq!(host.writes.get(), 0);

        host.remove(&["cb1i1-lab", "cd1i2-lab"]);
        let stopped = check_lab_links(
            &host,
            &lab,
            &ends,
            &running(&[]),
            &UnreadableEnds::default(),
        )
        .await;
        assert_eq!(stopped.broken_nodes(), Vec::<RecordId>::new());
    }

    #[test]
    fn private_bridge_016_an_older_releases_member_is_not_repaired_and_is_logged_once() {
        let lab = bridged_ctr1_lab();
        let ends = bridged_ctr1_ends();
        let mut host = FakeHost::default();
        host.container_interfaces
            .insert(("ctr1-lab".to_string(), "eth1".to_string()));
        let unreadable = UnreadableEnds::default();
        let all = running(&["ctr1"]);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (broken, log) = capture_log(|| {
            (0..3)
                .map(|_| {
                    let check =
                        runtime.block_on(check_lab_links(&host, &lab, &ends, &all, &unreadable));
                    check.broken_nodes()
                })
                .collect::<Vec<Vec<RecordId>>>()
        });
        assert_eq!(broken, vec![Vec::<RecordId>::new(); 3]);
        assert_eq!(host.writes.get(), 0);
        let older: Vec<&str> = log
            .lines()
            .filter(|line| line.contains("older release"))
            .collect();
        assert_eq!(older.len(), 1, "{log}");
        assert!(older[0].contains(" INFO "), "{log}");
        assert!(!log.contains(" WARN "), "{log}");
    }

    #[tokio::test]
    async fn link_reattach_023_a_bare_end_whose_peer_is_down_waits_for_the_peer() {
        // vm1 was started outside Sherpa while vm2 is shut off: nothing can be attached, so it
        // is not repaired on every scan.
        let (host, lab) = wired_lab();
        host.remove(&["tpb0"]);
        host.make(&["tpa0"]);
        let scan = scan(&host, &lab, &running(&["vm1", "ctr1"]), &[]).await;
        assert_eq!(scan.repaired, Vec::<RecordId>::new());
        assert_eq!(scan.states[&0], peer_down("vm2"));
        // vm1's other link has a bare end too, and its peer is up: that one is wired.
        host.make(&["tpa1"]);
        let scan = self::scan(&host, &lab, &running(&["vm1", "ctr1"]), &[]).await;
        assert_eq!(scan.repaired, vec![id("vm1")]);
        assert!(host.wired("tpa1", "tpb1"));
        assert_eq!(scan.states[&0], peer_down("vm2"));
        assert_eq!(scan.states[&1], LinkState::Attached);
    }

    #[tokio::test]
    async fn link_reattach_032_one_scan_repairs_with_nothing_remembered_and_the_next_nothing() {
        // As after a sherpad restart: a change made while it was down is seen from the kernel.
        let (host, lab) = wired_lab();
        host.make(&["tpb0"]);
        host.remove(&["tpb1", "cd1i2-lab"]);
        let all = running(&["vm1", "vm2", "ctr1"]);
        let first = scan(&host, &lab, &all, &[("ctr1", ctr1_end)]).await;
        assert_eq!(first.repaired, vec![id("vm2"), id("ctr1")]);
        host.writes.set(0);
        let second = scan(&host, &lab, &all, &[("ctr1", ctr1_end)]).await;
        assert_eq!(second.repaired, Vec::<RecordId>::new());
        assert_eq!(host.writes.get(), 0);
        assert_eq!(
            second.states,
            HashMap::from([(0, LinkState::Attached), (1, LinkState::Attached)])
        );
    }

    #[tokio::test]
    async fn link_reattach_024_a_repair_that_fails_stops_no_other_and_is_tried_again() {
        let (mut host, lab) = wired_lab();
        host.failing_redirects.insert("tpb0".to_string());
        host.make(&["tpa0", "tpa1"]);
        let all = running(&["vm1", "vm2", "ctr1"]);
        let first = scan(&host, &lab, &all, &[]).await;
        assert!(
            matches!(first.states[&0], LinkState::Failed { .. }),
            "{:?}",
            first.states
        );
        assert_eq!(first.states[&1], LinkState::Attached);
        assert!(host.wired("tpa1", "tpb1"));

        // The failed attach left vm2's end bare, so the next scan tries again.
        host.failing_redirects.clear();
        let second = scan(&host, &lab, &all, &[]).await;
        assert_eq!(second.faults, vec!["its end tpb0 has no redirect"]);
        assert!(host.wired("tpa0", "tpb0"));
        assert_eq!(second.states[&0], LinkState::Attached);
    }

    #[tokio::test]
    async fn link_reattach_025_a_missing_tap_or_an_unreadable_end_fails_the_link_with_no_repair() {
        {
            let (host, lab) = wired_lab();
            host.remove(&["tpb0"]);
            let scan = scan(&host, &lab, &running(&["vm1", "vm2", "ctr1"]), &[]).await;
            assert_eq!(scan.faults, vec!["its end tpb0 does not exist"]);
            assert_eq!(
                scan.states[&0],
                LinkState::Failed {
                    reason: "its end tpb0 does not exist".to_string()
                }
            );
        }

        {
            let (mut host, lab) = wired_lab();
            host.failing_filter_reads.insert("tpb0".to_string());
            let scan = scan(&host, &lab, &running(&["vm1", "vm2", "ctr1"]), &[]).await;
            assert_eq!(scan.repaired, Vec::<RecordId>::new());
            assert_eq!(host.writes.get(), 0);
            assert!(matches!(scan.states[&0], LinkState::Failed { .. }));
            assert_eq!(scan.states[&1], LinkState::Attached);
        }
    }

    #[test]
    fn link_reattach_033_an_unreadable_end_is_logged_once_and_once_when_it_can_be_read_again() {
        let (mut host, lab) = wired_lab();
        let ends = UnreadableEnds::default();
        let nodes = running(&["vm1", "vm2", "ctr1"]);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        host.failing_filter_reads.insert("tpb0".to_string());
        let (reasons, unreadable_log) = capture_log(|| {
            (0..3)
                .map(|_| {
                    let check = runtime.block_on(check_lab_links(&host, &lab, &[], &nodes, &ends));
                    check.into_states()[&0].clone()
                })
                .collect::<Vec<LinkState>>()
        });
        host.failing_filter_reads.clear();
        let (states, readable_log) = capture_log(|| {
            (0..2)
                .map(|_| {
                    let check = runtime.block_on(check_lab_links(&host, &lab, &[], &nodes, &ends));
                    check.into_states()[&0].clone()
                })
                .collect::<Vec<LinkState>>()
        });

        let warns: Vec<&str> = unreadable_log
            .lines()
            .filter(|line| line.contains(" WARN "))
            .collect();
        assert_eq!(warns.len(), 1, "{unreadable_log}");
        assert!(
            warns[0].contains("Failed to read the filter of P2p link end tpb0"),
            "{unreadable_log}"
        );
        assert!(!unreadable_log.contains(" ERROR "), "{unreadable_log}");
        let reference = warns[0]
            .split("reference=")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .unwrap();
        for state in &reasons {
            assert_eq!(
                *state,
                LinkState::Failed {
                    reason: format!(
                        "failed to read the filter of tpb0: internal error (ref {reference}); \
                         see the server log"
                    )
                }
            );
        }

        let infos: Vec<&str> = readable_log
            .lines()
            .filter(|line| line.contains(" INFO ") && line.contains("can be read again"))
            .collect();
        assert_eq!(infos.len(), 1, "{readable_log}");
        assert!(!readable_log.contains("reference="), "{readable_log}");
        assert_eq!(states, vec![LinkState::Attached, LinkState::Attached]);
    }

    // Integration: real kernel, Docker and libvirt, as root.

    /// Whether interface `iface` has the redirect program on its ingress.
    async fn has_filter(iface: &str) -> bool {
        let ifindex = network::get_ifindex(iface).await.unwrap();
        network::has_p2p_redirect(ifindex).await.unwrap()
    }

    /// Whether a ping from `container` reaches `address`.
    async fn pings(docker: &Docker, container: &str, address: &str) -> bool {
        exec_container(
            docker,
            container,
            vec!["ping", "-c", "2", "-W", "1", address],
        )
        .await
        .is_ok()
    }

    async fn start_alpine(docker: &Docker, name: &str) {
        let _ = docker
            .remove_container(
                name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..RemoveContainerOptions::default()
                }),
            )
            .await;
        docker
            .create_container(
                Some(CreateContainerOptions {
                    name: Some(name.to_string()),
                    ..CreateContainerOptions::default()
                }),
                ContainerCreateBody {
                    image: Some("alpine:latest".to_string()),
                    cmd: Some(vec!["sleep".to_string(), "infinity".to_string()]),
                    host_config: Some(HostConfig {
                        cap_add: Some(vec!["NET_ADMIN".to_string()]),
                        ..HostConfig::default()
                    }),
                    ..ContainerCreateBody::default()
                },
            )
            .await
            .unwrap();
        docker
            .start_container(name, None::<StartContainerOptions>)
            .await
            .unwrap();
    }

    /// A container's veth on the link: its first data interface, `eth1`.
    fn alpine_end(host_veth: &str) -> ContainerVeth {
        ContainerVeth {
            host_veth: host_veth.to_string(),
            host_alias: format!("it-{host_veth}"),
            interface_idx: 1,
            model_kind: ModelKind::default_for(NodeModel::AlpineLinux),
            admin_down: false,
            bridge: None,
        }
    }

    /// Two alpine containers joined by a P2p link through `KernelHost`, as `up` makes it.
    struct ContainerPair {
        docker: Docker,
        qemu: Arc<Qemu>,
        lab: LabLinks,
    }

    const CTR_A: &str = "it-repair-ctr1";
    const CTR_B: &str = "it-repair-ctr2";
    const TAP_A: &str = "it-rpa0";
    const TAP_B: &str = "it-rpb0";

    fn end_a() -> RecreatedEnd {
        RecreatedEnd::Container {
            container: CTR_A.to_string(),
            veths: vec![alpine_end(TAP_A)],
        }
    }

    fn end_b() -> RecreatedEnd {
        RecreatedEnd::Container {
            container: CTR_B.to_string(),
            veths: vec![alpine_end(TAP_B)],
        }
    }

    impl ContainerPair {
        async fn up() -> Self {
            let docker = Docker::connect_with_local_defaults().unwrap();
            start_alpine(&docker, CTR_A).await;
            start_alpine(&docker, CTR_B).await;
            let mut link = link(0, "ctr1", "ctr2");
            link.int_a = "eth1".to_string();
            link.tap_a = TAP_A.to_string();
            link.tap_b = TAP_B.to_string();
            let lab = LabLinks::new(
                "lab",
                vec![link],
                &[node("ctr1"), node("ctr2")],
                HashMap::from([
                    (id("ctr1"), NodeKind::Container),
                    (id("ctr2"), NodeKind::Container),
                ]),
            );
            let qemu = Arc::new(Qemu::default());
            let host = KernelHost::new(&docker, &qemu);
            for (node, end) in [("ctr1", end_a()), ("ctr2", end_b())] {
                let outcomes = repair_node(&host, &lab, &id(node), end).await;
                assert!(!outcomes[0].report.failed(), "{outcomes:?}");
            }
            let pair = Self { docker, qemu, lab };
            pair.address(CTR_A, "10.250.0.1/30").await;
            pair.address(CTR_B, "10.250.0.2/30").await;
            assert!(pair.pings().await, "the link carries traffic");
            pair
        }

        /// Give `container`'s link interface the address `cidr`, which a new namespace lacks.
        async fn address(&self, container: &str, cidr: &str) {
            exec_container(
                &self.docker,
                container,
                vec!["ip", "addr", "add", cidr, "dev", "eth1"],
            )
            .await
            .unwrap();
        }

        /// Whether a ping crosses the link from the first container to the second. Each
        /// forgets its neighbours first: a restarted container's interface has a new MAC
        /// address, which its peer learns only once its stale entry is probed again.
        async fn pings(&self) -> bool {
            for container in [CTR_A, CTR_B] {
                exec_container(&self.docker, container, vec!["ip", "neigh", "flush", "all"])
                    .await
                    .unwrap();
            }
            pings(&self.docker, CTR_A, "10.250.0.2").await
        }

        /// One scan of the real host, with both containers' ends planned as the scanner does.
        async fn scan(&self, running: &HashSet<RecordId>) -> HashMap<u16, LinkState> {
            let host = KernelHost::new(&self.docker, &self.qemu);
            let mut check =
                check_lab_links(&host, &self.lab, &[], running, &UnreadableEnds::default()).await;
            for node in check.broken_nodes() {
                let end = if node == id("ctr1") { end_a() } else { end_b() };
                let outcomes = repair_node(&host, &self.lab, &node, end).await;
                check.record(&outcomes);
            }
            check.into_states()
        }

        async fn down(self) {
            for name in [CTR_A, CTR_B] {
                let _ = self
                    .docker
                    .remove_container(
                        name,
                        Some(RemoveContainerOptions {
                            force: true,
                            ..RemoveContainerOptions::default()
                        }),
                    )
                    .await;
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires root and Docker with alpine:latest: makes containers, veths and eBPF programs"]
    async fn link_reattach_016_a_docker_restart_is_repaired_by_one_scan() {
        let pair = ContainerPair::up().await;
        pair.docker
            .restart_container(CTR_B, None::<RestartContainerOptions>)
            .await
            .unwrap();
        assert!(
            network::find_ifindex(TAP_B).await.unwrap().is_none(),
            "the restart took the veth"
        );

        let states = pair.scan(&running(&["ctr1", "ctr2"])).await;
        assert_eq!(states[&0], LinkState::Attached, "{states:?}");
        assert!(has_filter(TAP_A).await && has_filter(TAP_B).await);
        pair.address(CTR_B, "10.250.0.2/30").await;
        assert!(pair.pings().await, "traffic crosses again");
        pair.down().await;
    }

    #[tokio::test]
    #[ignore = "requires root and Docker with alpine:latest: makes containers, veths and eBPF programs"]
    async fn link_reattach_017_a_docker_stop_waits_and_the_start_is_repaired_by_one_scan() {
        let pair = ContainerPair::up().await;
        pair.docker
            .stop_container(CTR_B, None::<StopContainerOptions>)
            .await
            .unwrap();
        let states = pair.scan(&running(&["ctr1"])).await;
        assert_eq!(states[&0], peer_down("ctr2"));

        pair.docker
            .start_container(CTR_B, None::<StartContainerOptions>)
            .await
            .unwrap();
        let states = pair.scan(&running(&["ctr1", "ctr2"])).await;
        assert_eq!(states[&0], LinkState::Attached, "{states:?}");
        assert!(has_filter(TAP_A).await && has_filter(TAP_B).await);
        pair.address(CTR_B, "10.250.0.2/30").await;
        assert!(pair.pings().await, "traffic crosses again");
        pair.down().await;
    }

    #[tokio::test]
    #[ignore = "requires root and Docker with alpine:latest: makes containers, veths and eBPF programs"]
    async fn link_reattach_019_the_first_scan_repairs_what_changed_while_sherpad_was_down() {
        // As after a sherpad restart: the container was restarted while nothing was scanning,
        // and the check is made by a new host with no state.
        let pair = ContainerPair::up().await;
        pair.docker
            .restart_container(CTR_A, None::<RestartContainerOptions>)
            .await
            .unwrap();
        let states = pair.scan(&running(&["ctr1", "ctr2"])).await;
        assert_eq!(states[&0], LinkState::Attached, "{states:?}");
        let states = pair.scan(&running(&["ctr1", "ctr2"])).await;
        assert_eq!(
            states[&0],
            LinkState::Attached,
            "a second scan finds nothing to do"
        );
        pair.address(CTR_A, "10.250.0.1/30").await;
        assert!(pair.pings().await);
        pair.down().await;
    }

    const BR_CTR_A: &str = "it-bridge-ctr1";
    const BR_CTR_B: &str = "it-bridge-ctr2";
    const BRIDGE: &str = "it-brs0";

    fn bridge_member(container: &str, host_veth: &str) -> RecreatedEnd {
        RecreatedEnd::Container {
            container: container.to_string(),
            veths: vec![ContainerVeth {
                bridge: Some(BRIDGE.to_string()),
                ..alpine_end(host_veth)
            }],
        }
    }

    fn bridged_end(node: &RecordId) -> RecreatedEnd {
        if *node == id("ctr1") {
            bridge_member(BR_CTR_A, "it-cb1")
        } else {
            bridge_member(BR_CTR_B, "it-cb2")
        }
    }

    struct BridgedPair {
        docker: Docker,
        qemu: Arc<Qemu>,
        lab: LabLinks,
        ends: Vec<BridgeEnd>,
    }

    impl BridgedPair {
        async fn up() -> Self {
            let docker = Docker::connect_with_local_defaults().unwrap();
            let _ = network::delete_interface(BRIDGE).await;
            network::create_bridge(BRIDGE, "it-bridge").await.unwrap();
            start_alpine(&docker, BR_CTR_A).await;
            start_alpine(&docker, BR_CTR_B).await;
            let lab = LabLinks::new(
                "lab",
                vec![],
                &[node("ctr1"), node("ctr2")],
                HashMap::from([
                    (id("ctr1"), NodeKind::Container),
                    (id("ctr2"), NodeKind::Container),
                ]),
            );
            let pair = Self {
                docker,
                qemu: Arc::new(Qemu::default()),
                lab,
                ends: vec![
                    BridgeEnd {
                        node: id("ctr1"),
                        host_veth: "it-cb1".to_string(),
                        container: BR_CTR_A.to_string(),
                        interface: "eth1".to_string(),
                    },
                    BridgeEnd {
                        node: id("ctr2"),
                        host_veth: "it-cb2".to_string(),
                        container: BR_CTR_B.to_string(),
                        interface: "eth1".to_string(),
                    },
                ],
            };
            let host = KernelHost::new(&pair.docker, &pair.qemu);
            for node in ["ctr1", "ctr2"] {
                repair_node(&host, &pair.lab, &id(node), bridged_end(&id(node))).await;
            }
            pair.address(BR_CTR_A, "10.250.1.1/24").await;
            pair.address(BR_CTR_B, "10.250.1.2/24").await;
            pair
        }

        async fn address(&self, container: &str, cidr: &str) {
            exec_container(
                &self.docker,
                container,
                vec!["ip", "addr", "add", cidr, "dev", "eth1"],
            )
            .await
            .unwrap();
        }

        async fn pings(&self) -> bool {
            for container in [BR_CTR_A, BR_CTR_B] {
                exec_container(&self.docker, container, vec!["ip", "neigh", "flush", "all"])
                    .await
                    .unwrap();
            }
            pings(&self.docker, BR_CTR_A, "10.250.1.2").await
        }

        async fn scan(&self) -> Vec<RecordId> {
            let host = KernelHost::new(&self.docker, &self.qemu);
            let check = check_lab_links(
                &host,
                &self.lab,
                &self.ends,
                &running(&["ctr1", "ctr2"]),
                &UnreadableEnds::default(),
            )
            .await;
            let broken = check.broken_nodes();
            for node in &broken {
                repair_node(&host, &self.lab, node, bridged_end(node)).await;
            }
            broken
        }

        async fn down(self) {
            for name in [BR_CTR_A, BR_CTR_B] {
                let _ = self
                    .docker
                    .remove_container(
                        name,
                        Some(RemoveContainerOptions {
                            force: true,
                            ..RemoveContainerOptions::default()
                        }),
                    )
                    .await;
            }
            let _ = network::delete_interface(BRIDGE).await;
        }
    }

    #[tokio::test]
    #[ignore = "requires root and Docker with alpine:latest: makes containers, a bridge and veths"]
    async fn private_bridge_012_a_bridge_of_containers_on_no_link_carries_traffic_between_them() {
        let pair = BridgedPair::up().await;
        let shown = String::from_utf8(
            std::process::Command::new("ip")
                .args(["link", "show", BRIDGE])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        assert!(!shown.contains("NO-CARRIER"), "{shown}");
        assert!(pair.pings().await, "the members reach each other");
        pair.down().await;
    }

    #[tokio::test]
    #[ignore = "requires root and Docker with alpine:latest: makes containers, a bridge and veths"]
    async fn private_bridge_015_a_docker_restart_of_a_member_on_no_link_is_repaired_by_one_scan() {
        let pair = BridgedPair::up().await;
        pair.docker
            .restart_container(BR_CTR_B, None::<RestartContainerOptions>)
            .await
            .unwrap();
        assert!(
            network::find_ifindex("it-cb2").await.unwrap().is_none(),
            "the restart took the veth"
        );

        assert_eq!(pair.scan().await, vec![id("ctr2")]);
        assert_eq!(pair.scan().await, Vec::<RecordId>::new());
        pair.address(BR_CTR_B, "10.250.1.2/24").await;
        assert!(pair.pings().await, "traffic crosses again");
        pair.down().await;
    }

    /// A diskless domain with one `ethernet` interface on tap `tap`, as Sherpa's VMs have.
    fn domain_xml(name: &str, tap: &str) -> String {
        format!(
            "<domain type='kvm'><name>{name}</name><memory unit='MiB'>64</memory><vcpu>1</vcpu>\
             <os><type arch='x86_64'>hvm</type></os><devices>\
             <interface type='ethernet'><target dev='{tap}' managed='yes'/><model type='virtio'/></interface>\
             </devices></domain>"
        )
    }

    #[tokio::test]
    #[ignore = "requires root and libvirt: makes a domain, a veth and eBPF programs"]
    async fn link_reattach_018_a_vm_destroyed_and_started_with_virsh_is_repaired_by_one_scan() {
        let (vm, tap, peer) = ("it-repair-vm1", "it-rvm0", "it-rpeer0");
        let mut conn = Connect::open(Some("qemu:///system")).unwrap();
        if let Ok(old) = Domain::lookup_by_name(&conn, vm) {
            let _ = old.destroy();
            let _ = old.undefine();
        }
        let domain = Domain::define_xml(&conn, &domain_xml(vm, tap)).unwrap();
        domain.create().unwrap();
        let _ = network::delete_interface(peer).await;
        network::create_veth_pair(peer, &format!("{peer}p"), peer, peer)
            .await
            .unwrap();

        let mut link = link(0, "vm1", "peer");
        link.tap_a = tap.to_string();
        link.tap_b = peer.to_string();
        let lab = LabLinks::new(
            "lab",
            vec![link],
            &[node("vm1"), node("peer")],
            HashMap::new(),
        );
        let docker = Docker::connect_with_local_defaults().unwrap();
        let qemu = Arc::new(Qemu::default());
        let host = KernelHost::new(&docker, &qemu);
        let both = running(&["vm1", "peer"]);
        let scan_once = || async {
            let mut check =
                check_lab_links(&host, &lab, &[], &both, &UnreadableEnds::default()).await;
            let broken = check.broken_nodes();
            for node in &broken {
                let outcomes = repair_node(&host, &lab, node, RecreatedEnd::Domain).await;
                check.record(&outcomes);
            }
            (broken, check.into_states())
        };
        let (broken, states) = scan_once().await;
        assert_eq!(broken.len(), 2, "both new ends are bare");
        assert_eq!(states[&0], LinkState::Attached);
        let old_index = network::get_ifindex(tap).await.unwrap();

        domain.destroy().unwrap();
        domain.create().unwrap();
        let new_index = network::get_ifindex(tap).await.unwrap();
        assert_ne!(old_index, new_index, "libvirt made a new tap");
        assert!(!has_filter(tap).await);

        let (broken, states) = scan_once().await;
        assert_eq!(broken, vec![id("vm1")]);
        assert_eq!(states[&0], LinkState::Attached);
        assert!(has_filter(tap).await && has_filter(peer).await);
        let (broken, _) = scan_once().await;
        assert_eq!(
            broken,
            Vec::<RecordId>::new(),
            "a second scan finds nothing to do"
        );

        domain.destroy().unwrap();
        domain.undefine().unwrap();
        network::delete_interface(peer).await.unwrap();
        conn.close().unwrap();
    }

    // Integration: carrier, seen from inside a real VM guest and a container's namespace. Needs
    // root, Docker with alpine:latest, libvirt, and the alpine_linux VM image.

    /// The first-boot script of the carrier test VM: it installs, and starts, a per-boot script
    /// that prints the carrier of the guest's `eth0` to its serial port every second.
    const CARRIER_GUEST_SCRIPT: &str = r#"#!/bin/sh
mkdir -p /var/lib/cloud/scripts/per-boot
cat > /var/lib/cloud/scripts/per-boot/carrier.sh <<'END'
#!/bin/sh
ip link set eth0 up
( while true; do echo "SHERPA-CARRIER $(cat /sys/class/net/eth0/carrier)"; sleep 1; done ) >/dev/ttyS0 2>&1 </dev/null &
END
chmod +x /var/lib/cloud/scripts/per-boot/carrier.sh
/var/lib/cloud/scripts/per-boot/carrier.sh
"#;

    /// A VM booted from the `alpine_linux` image with one `ethernet` interface on tap `tap`, its
    /// `NoCloud` seed on a CD, and its serial port written to `serial`.
    fn guest_domain_xml(name: &str, disk: &str, seed: &str, tap: &str, serial: &str) -> String {
        format!(
            "<domain type='kvm'><name>{name}</name><memory unit='MiB'>256</memory><vcpu>1</vcpu>\
             <os><type arch='x86_64' machine='q35'>hvm</type><boot dev='hd'/></os>\
             <features><acpi/><apic/></features><cpu mode='host-model'/><devices>\
             <disk type='file' device='disk'><driver name='qemu' type='qcow2'/>\
             <source file='{disk}'/><target dev='vda' bus='virtio'/></disk>\
             <disk type='file' device='cdrom'><driver name='qemu' type='raw'/>\
             <source file='{seed}'/><target dev='sda' bus='sata'/><readonly/></disk>\
             <interface type='ethernet'><target dev='{tap}' managed='yes'/><model type='virtio'/></interface>\
             <serial type='file'><source path='{serial}'/><target port='0'/></serial>\
             </devices></domain>"
        )
    }

    /// The `alpine_linux` VM image: the first version installed.
    fn alpine_vm_image() -> String {
        let dir = format!("{SHERPA_IMAGES_PATH}/alpine_linux");
        let version = fs::read_dir(&dir)
            .expect("the alpine_linux image is installed")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .min()
            .expect("an alpine_linux version");
        format!("{dir}/{version}/virtioa.qcow2")
    }

    /// A VM (`vm1`) and an alpine container (`ctr1`) joined by a P2p link, checked by scans
    /// as the scanner makes them, with each link's state kept between scans.
    struct GuestPair {
        lab_id: &'static str,
        docker: Docker,
        qemu: Arc<Qemu>,
        conn: Connect,
        domain: Domain,
        lab: LabLinks,
        states: LinkStates,
        container: String,
        veth: String,
        tap: String,
        serial: String,
        disk: String,
        seed: String,
    }

    impl GuestPair {
        /// Start both nodes, and wire their link with one scan.
        async fn up(lab_id: &'static str, tap: &str, veth: &str) -> Self {
            let docker = Docker::connect_with_local_defaults().unwrap();
            let container = format!("ctr1-{lab_id}");
            start_alpine(&docker, &container).await;

            let conn = Connect::open(Some("qemu:///system")).unwrap();
            let name = format!("vm1-{lab_id}");
            if let Ok(old) = Domain::lookup_by_name(&conn, &name) {
                let _ = old.destroy();
                let _ = old.undefine();
            }
            let disk_name = format!("{name}-it.qcow2");
            let disk = format!("{SHERPA_STORAGE_POOL_PATH}/{disk_name}");
            let _ = libvirt::delete_disk(&conn, &disk_name);
            libvirt::create_overlay(&conn, &alpine_vm_image(), &disk, None).unwrap();
            let seed_dir = env::temp_dir().join(format!("{name}-seed"));
            let _ = fs::remove_dir_all(&seed_dir);
            fs::create_dir_all(&seed_dir).unwrap();
            fs::write(seed_dir.join("user-data"), CARRIER_GUEST_SCRIPT).unwrap();
            fs::write(seed_dir.join("meta-data"), format!("instance-id: {name}\n")).unwrap();
            let seed = format!("{SHERPA_STORAGE_POOL_PATH}/{name}-seed.iso");
            iso::create_iso(&seed_dir, &seed, "cidata").unwrap();
            fs::remove_dir_all(&seed_dir).unwrap();
            let serial = format!("/var/log/libvirt/qemu/{name}-serial.log");
            let _ = fs::remove_file(&serial);
            let domain =
                Domain::define_xml(&conn, &guest_domain_xml(&name, &disk, &seed, tap, &serial))
                    .unwrap();
            domain.create().unwrap();

            let mut link = link(0, "vm1", "ctr1");
            link.int_a = "eth0".to_string();
            link.tap_a = tap.to_string();
            link.tap_b = veth.to_string();
            let lab = LabLinks::new(
                lab_id,
                vec![link],
                &[node("vm1"), node("ctr1")],
                HashMap::from([
                    (id("vm1"), NodeKind::VirtualMachine),
                    (id("ctr1"), NodeKind::Container),
                ]),
            );
            let pair = Self {
                lab_id,
                docker,
                qemu: Arc::new(Qemu::default()),
                domain,
                lab,
                states: LinkStates::default(),
                container,
                veth: veth.to_string(),
                tap: tap.to_string(),
                serial,
                disk: disk_name,
                seed,
                conn,
            };
            assert_eq!(
                pair.scan(&["vm1", "ctr1"]).await,
                LinkState::Attached,
                "the first scan wires the link"
            );
            pair.wait_for_guest_carrier(true).await;
            pair.wait_for_container_carrier(true).await;
            pair
        }

        /// One scan, as the scanner makes it: check, repair, carrier, record. Returns the
        /// link's state.
        async fn scan(&self, running_nodes: &[&str]) -> LinkState {
            let host = KernelHost::new(&self.docker, &self.qemu);
            let mut check = check_lab_links(
                &host,
                &self.lab,
                &[],
                &running(running_nodes),
                &UnreadableEnds::default(),
            )
            .await;
            for node in check.broken_nodes() {
                let end = if node == id("ctr1") {
                    RecreatedEnd::Container {
                        container: self.container.clone(),
                        veths: vec![alpine_end(&self.veth)],
                    }
                } else {
                    RecreatedEnd::Domain
                };
                let outcomes = repair_node(&host, &self.lab, &node, end).await;
                check.record(&outcomes);
            }
            let previous = self.states.of_lab(self.lab_id);
            let fresh = check.fresh_ends();
            let states = check.into_states();
            follow_carrier(&host, &self.lab, &previous, &states, &fresh).await;
            let state = states[&0].clone();
            self.states.replace_lab(self.lab_id, states);
            state
        }

        /// The carrier the guest last printed on its serial port.
        fn guest_carrier(&self) -> Option<bool> {
            let log = fs::read_to_string(Path::new(&self.serial)).unwrap_or_default();
            log.lines()
                .rev()
                .find_map(|line| line.trim().strip_prefix("SHERPA-CARRIER "))
                .map(|carrier| carrier.trim() == "1")
        }

        /// Wait up to three minutes, as the guest boots, for it to print carrier `up`.
        async fn wait_for_guest_carrier(&self, up: bool) {
            for _ in 0..180 {
                if self.guest_carrier() == Some(up) {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            panic!(
                "the guest's eth0 carrier is not {up}: it last printed {:?}",
                self.guest_carrier()
            );
        }

        /// Wait up to ten seconds for the container's `eth1` to have carrier `up`, read inside
        /// its network namespace.
        async fn wait_for_container_carrier(&self, up: bool) {
            let want = if up { "1" } else { "0" };
            let check = format!("grep -qx {want} /sys/class/net/eth1/carrier");
            for _ in 0..10 {
                if exec_container(&self.docker, &self.container, vec!["sh", "-c", &check])
                    .await
                    .is_ok()
                {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            panic!("the container's eth1 carrier is not {up}");
        }

        async fn down(self) {
            let _ = self.domain.destroy();
            let _ = self.domain.undefine();
            let _ = libvirt::delete_disk(&self.conn, &self.disk);
            let _ = fs::remove_file(&self.seed);
            let _ = fs::remove_file(&self.serial);
            let _ = self
                .docker
                .remove_container(
                    &self.container,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..RemoveContainerOptions::default()
                    }),
                )
                .await;
            let _ = network::delete_interface(&self.veth).await;
            let mut conn = self.conn;
            let _ = conn.close();
        }
    }

    #[tokio::test]
    #[ignore = "requires root, Docker with alpine:latest, libvirt and the alpine_linux image: boots a VM"]
    async fn link_carrier_001_a_stopped_container_takes_the_link_down_in_its_vm_peers_guest() {
        let pair = GuestPair::up("itcarr1", "itc1a0", "itc1b0").await;

        pair.docker
            .stop_container(&pair.container, None::<StopContainerOptions>)
            .await
            .unwrap();
        assert_eq!(pair.scan(&["vm1"]).await, peer_down("ctr1"));
        pair.wait_for_guest_carrier(false).await;
        assert_eq!(
            libvirt::interface_link_up(&pair.conn, "vm1-itcarr1", &pair.tap).unwrap(),
            Some(false),
            "the live domain has the link down"
        );

        pair.docker
            .start_container(&pair.container, None::<StartContainerOptions>)
            .await
            .unwrap();
        assert_eq!(pair.scan(&["vm1", "ctr1"]).await, LinkState::Attached);
        pair.wait_for_guest_carrier(true).await;
        pair.wait_for_container_carrier(true).await;

        // The next scan finds nothing changed.
        assert_eq!(pair.scan(&["vm1", "ctr1"]).await, LinkState::Attached);
        assert_eq!(pair.guest_carrier(), Some(true));
        pair.down().await;
    }

    #[tokio::test]
    #[ignore = "requires root, Docker with alpine:latest, libvirt and the alpine_linux image: boots a VM"]
    async fn link_carrier_002_a_stopped_vm_takes_carrier_from_its_container_peer() {
        let pair = GuestPair::up("itcarr2", "itc2a0", "itc2b0").await;

        pair.domain.destroy().unwrap();
        assert_eq!(pair.scan(&["ctr1"]).await, peer_down("vm1"));
        pair.wait_for_container_carrier(false).await;
        assert_eq!(network::link_is_up(&pair.veth).await.unwrap(), Some(false));

        let _ = fs::remove_file(&pair.serial);
        pair.domain.create().unwrap();
        assert_eq!(pair.scan(&["vm1", "ctr1"]).await, LinkState::Attached);
        pair.wait_for_container_carrier(true).await;
        pair.wait_for_guest_carrier(true).await;
        assert!(has_filter(&pair.tap).await && has_filter(&pair.veth).await);
        pair.down().await;
    }

    #[tokio::test]
    #[ignore = "requires root, Docker with alpine:latest, libvirt and the alpine_linux image: boots a VM"]
    async fn link_carrier_003_a_container_unpaused_by_resume_gives_its_vm_peer_carrier_at_once() {
        let pair = GuestPair::up("itcarr3", "itc3a0", "itc3b0").await;

        // `sherpa down -n ctr1` pauses the container; a scan takes the VM's link down.
        pair.docker.pause_container(&pair.container).await.unwrap();
        assert_eq!(pair.scan(&["vm1"]).await, peer_down("ctr1"));
        pair.wait_for_guest_carrier(false).await;

        // `sherpa resume -n ctr1` unpauses it, and decides its links at once.
        pair.docker
            .unpause_container(&pair.container)
            .await
            .unwrap();
        let started = Instant::now();
        let host = KernelHost::new(&pair.docker, &pair.qemu);
        let states = follow_started_links(
            &host,
            &pair.lab,
            &HashSet::from([id("ctr1")]),
            &query_runtimes(&pair.docker, &pair.qemu).await,
            &pair.states,
            &UnreadableEnds::default(),
        )
        .await;
        assert_eq!(states[&0], LinkState::Attached);
        assert_eq!(
            libvirt::interface_link_up(&pair.conn, "vm1-itcarr3", &pair.tap).unwrap(),
            Some(true),
            "the live domain has the link up when the call returns"
        );
        // The guest prints its carrier once a second.
        while pair.guest_carrier() != Some(true) && started.elapsed() < Duration::from_secs(10) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let elapsed = started.elapsed();
        println!("resume to guest carrier: {elapsed:?}");
        assert_eq!(pair.guest_carrier(), Some(true), "after {elapsed:?}");
        assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");

        // The next scan finds nothing changed.
        assert_eq!(pair.scan(&["vm1", "ctr1"]).await, LinkState::Attached);
        assert_eq!(pair.guest_carrier(), Some(true));
        pair.down().await;
    }
}
