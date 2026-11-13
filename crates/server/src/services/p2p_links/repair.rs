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

use std::collections::{HashMap, HashSet};
use std::fmt;

use tracing::instrument;

use super::host::LinkHost;
use super::reattach::{LabLinks, LinkOutcome, RecreatedEnd, reattach_node_links};
use super::veths::ContainerVeth;

use shared::data::{DbLink, LinkState, RecordId};

/// What is wrong with an end of a running node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EndFault {
    /// The end does not exist.
    Missing,
    /// The end exists with no redirect filter, and its peer's end exists.
    NoRedirect,
}

/// One check of a lab's links: the ends to repair, and the state of each link as the kernel
/// shows it, which a repair replaces with what it left the link in.
pub(crate) struct LinkCheck {
    /// The nodes with an end to repair, each once, in the order their ends were found.
    broken: Vec<RecordId>,
    states: HashMap<u16, LinkState>,
}

/// What the kernel shows of one end of a running node.
enum Seen {
    /// It exists, with its redirect or without.
    Present {
        redirect: bool,
    },
    Absent,
    /// It could not be read, and why.
    Unreadable(String),
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

/// Check each of the lab's links in the kernel. `running` holds the nodes whose runtime reports
/// them running: only their ends are checked. A link with an end to repair is `Failed` until
/// the repair's outcome is recorded.
#[instrument(skip_all, level = "debug")]
pub(crate) async fn check_lab_links(
    host: &impl LinkHost,
    lab: &LabLinks,
    running: &HashSet<RecordId>,
) -> LinkCheck {
    let mut links: Vec<&DbLink> = lab.links().iter().collect();
    links.sort_by_key(|link| link.index);
    let mut check = LinkCheck {
        broken: vec![],
        states: HashMap::new(),
    };
    for link in links {
        let state = check_link(host, lab, link, running, &mut check.broken).await;
        check.states.insert(link.index, state);
    }
    check
}

/// Check one link: add the nodes of its ends that need repair to `broken`, and return its state as the
/// kernel shows it.
async fn check_link(
    host: &impl LinkHost,
    lab: &LabLinks,
    link: &DbLink,
    running: &HashSet<RecordId>,
    broken: &mut Vec<RecordId>,
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
        let fault = match see(host, iface).await {
            Seen::Absent => Some(EndFault::Missing),
            Seen::Present { redirect: false } if exists(host, peer_iface).await => {
                Some(EndFault::NoRedirect)
            }
            Seen::Present { .. } => None,
            Seen::Unreadable(reason) => {
                unreadable = Some(reason);
                None
            }
        };
        if let Some(fault) = fault {
            tracing::info!(node = %lab.name(node), link = %lab.label(link), %iface, %fault, "P2p link end needs repair");
            faults.push((iface, fault));
            if !broken.contains(node) {
                broken.push(node.clone());
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
        Err(error) => return Seen::Unreadable(format!("failed to look up {iface}: {error:#}")),
    };
    match host.has_redirect(ifindex).await {
        Ok(redirect) => Seen::Present { redirect },
        Err(error) => Seen::Unreadable(format!("failed to read the filter of {iface}: {error:#}")),
    }
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

    use bollard::Docker;
    use bollard::query_parameters::{
        CreateContainerOptions, RemoveContainerOptions, RestartContainerOptions,
        StartContainerOptions, StopContainerOptions,
    };
    use bollard::secret::{ContainerCreateBody, HostConfig};
    use container::exec_container;
    use virt::connect::Connect;
    use virt::domain::Domain;

    use super::super::fake::FakeHost;
    use super::super::host::KernelHost;
    use super::super::veths::ContainerVeth;
    use shared::data::{BridgeKind, DbNode, LinkImpairment, NodeModel, NodeState};

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
            vnc_port: None,
            ssh_host_key: None,
            cpu_pinning: None,
            qemu_thread_cpus: None,
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
        let lab = LabLinks::new(vec![link(0, "vm1", "vm2"), link(1, "vm1", "ctr1")], &nodes);
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
            node_model: NodeModel::FrrLinux,
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
        /// The nodes it repaired, in order.
        repaired: Vec<RecordId>,
        /// What the check found wrong with each link it found an end to repair on, in index
        /// order.
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
        let mut check = check_lab_links(host, lab, running).await;
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
    async fn a_lab_nothing_changed_in_is_left_alone() {
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
    async fn a_vm_shut_off_outside_sherpa_is_not_repaired_and_its_links_wait() {
        // `virsh destroy` or `shutdown`: vm2's tap goes with its filter; vm1's end stays.
        let (host, lab) = wired_lab();
        host.remove(&["tpb0"]);
        let scan = scan(&host, &lab, &running(&["vm1", "ctr1"]), &[]).await;
        assert_eq!(scan.repaired, Vec::<RecordId>::new());
        assert_eq!(host.writes.get(), 0);
        assert_eq!(scan.states[&0], peer_down("vm2"));
        assert_eq!(scan.states[&1], LinkState::Attached);
    }

    #[tokio::test]
    async fn a_vm_started_outside_sherpa_has_its_bare_taps_wired() {
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
    async fn a_restarted_container_gets_its_missing_veths_back_and_its_links_wired() {
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
    async fn only_the_missing_veths_of_a_container_are_made() {
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

    #[tokio::test]
    async fn a_stopped_container_is_not_repaired_and_its_links_wait() {
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

    #[tokio::test]
    async fn a_paused_node_is_not_repaired() {
        // `virsh suspend` after the VM was started outside Sherpa: its tap is bare, but the
        // node is not running, so nothing is done to it.
        let (host, lab) = wired_lab();
        host.make(&["tpb0"]);
        let scan = scan(&host, &lab, &running(&["vm1", "ctr1"]), &[]).await;
        assert_eq!(scan.repaired, Vec::<RecordId>::new());
        assert_eq!(host.writes.get(), 0);
        assert_eq!(scan.states[&0], peer_down("vm2"));
    }

    #[tokio::test]
    async fn a_bare_end_whose_peer_is_down_waits_for_the_peer() {
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
    async fn the_first_scan_repairs_with_nothing_remembered_and_the_next_finds_nothing() {
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
    async fn a_repair_that_fails_does_not_stop_the_others_and_is_tried_again() {
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
    async fn a_running_vm_whose_tap_is_missing_fails_its_link() {
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

    #[tokio::test]
    async fn an_end_that_cannot_be_read_fails_its_link_without_a_repair() {
        let (mut host, lab) = wired_lab();
        host.failing_filter_reads.insert("tpb0".to_string());
        let scan = scan(&host, &lab, &running(&["vm1", "vm2", "ctr1"]), &[]).await;
        assert_eq!(scan.repaired, Vec::<RecordId>::new());
        assert_eq!(host.writes.get(), 0);
        assert!(matches!(scan.states[&0], LinkState::Failed { .. }));
        assert_eq!(scan.states[&1], LinkState::Attached);
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
            node_model: NodeModel::AlpineLinux,
            admin_down: false,
            bridge: None,
        }
    }

    /// Two alpine containers joined by a P2p link through `KernelHost`, as `up` makes it.
    struct ContainerPair {
        docker: Docker,
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
            let lab = LabLinks::new(vec![link], &[node("ctr1"), node("ctr2")]);
            let host = KernelHost::new(&docker);
            for (node, end) in [("ctr1", end_a()), ("ctr2", end_b())] {
                let outcomes = repair_node(&host, &lab, &id(node), end).await;
                assert!(!outcomes[0].report.failed(), "{outcomes:?}");
            }
            let pair = Self { docker, lab };
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
            let host = KernelHost::new(&self.docker);
            let mut check = check_lab_links(&host, &self.lab, running).await;
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
    async fn a_docker_restart_is_repaired_by_one_scan() {
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
    async fn a_docker_stop_waits_and_the_start_is_repaired_by_one_scan() {
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
    async fn a_scan_with_nothing_remembered_repairs_what_changed_before_it() {
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
    async fn a_vm_destroyed_and_started_outside_sherpa_is_repaired_by_one_scan() {
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
        let lab = LabLinks::new(vec![link], &[node("vm1"), node("peer")]);
        let docker = Docker::connect_with_local_defaults().unwrap();
        let host = KernelHost::new(&docker);
        let both = running(&["vm1", "peer"]);
        let scan_once = || async {
            let mut check = check_lab_links(&host, &lab, &both).await;
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
}
