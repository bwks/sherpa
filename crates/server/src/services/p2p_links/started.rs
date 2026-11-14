//! The links of nodes an operation started: their state is decided and their carrier set at
//! once, as the next scan would, so the scan finds nothing changed.
//!
//! Unpausing a node makes none of its ends again, so re-attach is not run for it and nothing
//! else would give its peers their carrier back before the next scan. A node booted from off
//! is re-attached first (`reattach_node_links`), which cannot tell a paused peer from a
//! running one: its peer's end exists either way. Both are checked here as the scanner checks
//! them (`check_lab_links`), with the nodes the runtimes report running, and each end's carrier
//! follows the change from the state last recorded (`follow_carrier`).
//!
//! A link the check finds `Failed` is left to the scan, which repairs what it can before it
//! records a state: recording the failure here would take both ends down until then.

use std::collections::{HashMap, HashSet};

use tracing::instrument;

use super::carrier::follow_carrier;
use super::host::LinkHost;
use super::reattach::LabLinks;
use super::repair::check_lab_links;
use crate::daemon::link_states::LinkStates;
use crate::services::node_lifecycle::Runtimes;

use shared::data::{LinkState, RecordId};

/// Decide the state of each link of the `started` nodes as a scan would, from the nodes
/// `runtimes` reports running; make each end's carrier follow it from the state `link_states`
/// holds, and record it. Returns the states recorded, by link index.
///
/// When libvirt or Docker did not answer, nothing is decided: a node of that runtime would be
/// taken as down, and its peers' links with it. The next scan decides them.
#[instrument(skip_all, fields(lab_id = %lab.lab_id()), level = "debug")]
pub(crate) async fn follow_started_links(
    host: &impl LinkHost,
    lab: &LabLinks,
    started: &HashSet<RecordId>,
    runtimes: &Runtimes,
    link_states: &LinkStates,
) -> HashMap<u16, LinkState> {
    if !runtimes.answered() {
        tracing::warn!("A runtime did not answer; the started nodes' links are left to the scan");
        return HashMap::new();
    }
    let links_of_started: HashSet<u16> = lab
        .links()
        .iter()
        .filter(|link| started.contains(&link.node_a) || started.contains(&link.node_b))
        .map(|link| link.index)
        .collect();
    if links_of_started.is_empty() {
        return HashMap::new();
    }
    let states: HashMap<u16, LinkState> = check_lab_links(host, lab, &lab.running(runtimes))
        .await
        .into_states()
        .into_iter()
        .filter(|(index, state)| {
            links_of_started.contains(index) && !matches!(state, LinkState::Failed { .. })
        })
        .collect();
    follow_carrier(
        host,
        lab,
        &link_states.of_lab(lab.lab_id()),
        &states,
        &HashSet::new(),
    )
    .await;
    for (index, state) in &states {
        link_states.record(lab.lab_id(), *index, state.clone());
    }
    states
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::fake::FakeHost;
    use super::super::host::EndDevice;
    use bollard::secret::ContainerStateStatusEnum;
    use shared::data::{BridgeKind, DbLink, DbNode, LinkImpairment, NodeKind, NodeState};
    use virt::sys::{VIR_DOMAIN_RUNNING, VIR_DOMAIN_SHUTOFF};

    fn id(name: &str) -> RecordId {
        RecordId::new("node", name)
    }

    fn ids(names: &[&str]) -> HashSet<RecordId> {
        names.iter().map(|name| id(name)).collect()
    }

    fn node(name: &str) -> DbNode {
        DbNode {
            id: Some(id(name)),
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
            node_a: id(node_a),
            node_b: id(node_b),
            int_a: format!("eth{}", index + 1),
            int_b: "eth1".to_string(),
            lab: RecordId::new("lab", "lab"),
            tap_a: format!("tpa{index}"),
            tap_b: format!("tpb{index}"),
            impairment_a_to_b: LinkImpairment::default(),
            impairment_b_to_a: LinkImpairment::default(),
        }
    }

    /// vm1 linked to ctr1 (0) and vm2 linked to ctr2 (1), every end made and wired, with
    /// carrier.
    fn lab() -> (FakeHost, LabLinks) {
        let nodes = ["vm1", "ctr1", "vm2", "ctr2"].map(node);
        let kinds = HashMap::from([
            (id("vm1"), NodeKind::VirtualMachine),
            (id("ctr1"), NodeKind::Container),
            (id("vm2"), NodeKind::VirtualMachine),
            (id("ctr2"), NodeKind::Container),
        ]);
        let lab = LabLinks::new(
            "lab",
            vec![link(0, "vm1", "ctr1"), link(1, "vm2", "ctr2")],
            &nodes,
            kinds,
        );
        let host = FakeHost::default();
        host.make(&["tpa0", "tpb0", "tpa1", "tpb1"]);
        for (a, b) in [("tpa0", "tpb0"), ("tpa1", "tpb1")] {
            let a_index = host.index(a).unwrap();
            let b_index = host.index(b).unwrap();
            host.redirects.borrow_mut().insert(a.to_string(), b_index);
            host.redirects.borrow_mut().insert(b.to_string(), a_index);
        }
        (host, lab)
    }

    /// What the runtimes report when the nodes in `running` run and the others are paused.
    fn runtimes(running: &[&str]) -> Runtimes {
        let state = |name: &str| {
            if running.contains(&name) {
                ContainerStateStatusEnum::RUNNING
            } else {
                ContainerStateStatusEnum::PAUSED
            }
        };
        let domain = |name: &str| {
            if running.contains(&name) {
                VIR_DOMAIN_RUNNING
            } else {
                VIR_DOMAIN_SHUTOFF
            }
        };
        Runtimes {
            domains: Some(
                ["vm1", "vm2"]
                    .map(|name| (format!("{name}-lab"), domain(name)))
                    .into(),
            ),
            containers: Some(
                ["ctr1", "ctr2"]
                    .map(|name| (format!("{name}-lab"), state(name)))
                    .into(),
            ),
        }
    }

    fn peer_down(peer: &str) -> LinkState {
        LinkState::PeerDown {
            peer: peer.to_string(),
        }
    }

    fn vm_tap(domain: &str, tap: &str) -> EndDevice {
        EndDevice::Tap {
            domain: domain.to_string(),
            tap: tap.to_string(),
        }
    }

    /// One scan's link check and carrier, as `scanner::repair_links` makes it with nothing to
    /// repair: returns the carrier it changed.
    async fn scan(
        host: &FakeHost,
        lab: &LabLinks,
        running: &HashSet<RecordId>,
        link_states: &LinkStates,
    ) -> Vec<(EndDevice, bool)> {
        let check = check_lab_links(host, lab, running).await;
        let fresh = check.fresh_ends();
        let states = check.into_states();
        follow_carrier(host, lab, &link_states.of_lab("lab"), &states, &fresh).await;
        link_states.replace_lab("lab", states);
        host.take_carrier_writes()
    }

    #[tokio::test]
    async fn an_unpaused_container_gives_its_vm_peer_carrier_at_once() {
        // `sherpa down -n ctr1` paused the container, and a scan took vm1's link down.
        let (host, lab) = lab();
        let link_states = LinkStates::default();
        let writes = scan(&host, &lab, &ids(&["vm1", "vm2", "ctr2"]), &link_states).await;
        assert_eq!(writes, vec![(vm_tap("vm1-lab", "tpa0"), false)]);

        // `sherpa resume -n ctr1` unpauses it.
        let states = follow_started_links(
            &host,
            &lab,
            &ids(&["ctr1"]),
            &runtimes(&["vm1", "ctr1", "vm2", "ctr2"]),
            &link_states,
        )
        .await;
        assert_eq!(states, HashMap::from([(0, LinkState::Attached)]));
        assert_eq!(
            host.take_carrier_writes(),
            vec![(vm_tap("vm1-lab", "tpa0"), true)]
        );
        assert!(host.has_carrier("tpa0") && host.has_carrier("tpb0"));
        assert_eq!(link_states.of_lab("lab")[&0], LinkState::Attached);

        // The next scan finds nothing changed.
        let writes = scan(
            &host,
            &lab,
            &ids(&["vm1", "ctr1", "vm2", "ctr2"]),
            &link_states,
        )
        .await;
        assert_eq!(writes, vec![]);
    }

    #[tokio::test]
    async fn a_node_started_while_its_peer_is_down_leaves_its_link_down() {
        // `sherpa down`: vm1 shut off, ctr1 paused. The scan names vm1 and takes ctr1's end down.
        let (host, lab) = lab();
        host.remove(&["tpa0"]);
        let link_states = LinkStates::default();
        let writes = scan(&host, &lab, &ids(&["vm2", "ctr2"]), &link_states).await;
        assert_eq!(writes, vec![(EndDevice::Veth("tpb0".to_string()), false)]);

        // `sherpa resume -n ctr1`: vm1 is still off, so the state is the same and nothing moves.
        let states = follow_started_links(
            &host,
            &lab,
            &ids(&["ctr1"]),
            &runtimes(&["ctr1", "vm2", "ctr2"]),
            &link_states,
        )
        .await;
        assert_eq!(states, HashMap::from([(0, peer_down("vm1"))]));
        assert_eq!(host.take_carrier_writes(), vec![]);
        assert!(!host.has_carrier("tpb0"));
        let writes = scan(&host, &lab, &ids(&["ctr1", "vm2", "ctr2"]), &link_states).await;
        assert_eq!(writes, vec![]);
    }

    #[tokio::test]
    async fn a_vm_booted_beside_a_paused_peer_has_its_link_taken_down_at_once() {
        // vm1 was off and ctr1 is paused. Resume boots vm1, and its re-attach finds ctr1's end,
        // so it records the link attached and gives both ends carrier. ctr1 is not running.
        let (host, lab) = lab();
        let link_states = LinkStates::default();
        link_states.record("lab", 0, LinkState::Attached);
        let states = follow_started_links(
            &host,
            &lab,
            &ids(&["vm1"]),
            &runtimes(&["vm1", "vm2", "ctr2"]),
            &link_states,
        )
        .await;
        assert_eq!(states, HashMap::from([(0, peer_down("ctr1"))]));
        assert_eq!(
            host.take_carrier_writes(),
            vec![(vm_tap("vm1-lab", "tpa0"), false)]
        );
        let writes = scan(&host, &lab, &ids(&["vm1", "vm2", "ctr2"]), &link_states).await;
        assert_eq!(writes, vec![], "the scan finds the state recorded");
    }

    #[tokio::test]
    async fn only_the_links_of_the_started_nodes_are_decided() {
        // ctr2 is paused and link 1's state is recorded as before; resuming ctr1 leaves it.
        let (host, lab) = lab();
        let link_states = LinkStates::default();
        link_states.record("lab", 0, peer_down("ctr1"));
        link_states.record("lab", 1, LinkState::Attached);
        host.set_carrier_by_hand("tpa0", false);
        let states = follow_started_links(
            &host,
            &lab,
            &ids(&["ctr1"]),
            &runtimes(&["vm1", "ctr1", "vm2"]),
            &link_states,
        )
        .await;
        assert_eq!(states.keys().copied().collect::<Vec<_>>(), vec![0]);
        assert_eq!(link_states.of_lab("lab")[&1], LinkState::Attached);
        assert_eq!(
            host.take_carrier_writes(),
            vec![(vm_tap("vm1-lab", "tpa0"), true)]
        );
    }

    #[tokio::test]
    async fn a_link_with_an_end_to_repair_is_left_to_the_scan() {
        // ctr1's veth is gone while it runs: the scan repairs it. Nothing is recorded or set.
        let (host, lab) = lab();
        let link_states = LinkStates::default();
        link_states.record("lab", 0, peer_down("ctr1"));
        host.remove(&["tpb0"]);
        let states = follow_started_links(
            &host,
            &lab,
            &ids(&["ctr1"]),
            &runtimes(&["vm1", "ctr1", "vm2", "ctr2"]),
            &link_states,
        )
        .await;
        assert_eq!(states, HashMap::new());
        assert_eq!(host.take_carrier_writes(), vec![]);
        assert_eq!(link_states.of_lab("lab")[&0], peer_down("ctr1"));
    }

    #[tokio::test]
    async fn nothing_is_decided_when_a_runtime_did_not_answer() {
        // libvirt did not answer: vm1 would be taken as down, and ctr1's end with it.
        let (host, lab) = lab();
        let link_states = LinkStates::default();
        link_states.record("lab", 0, peer_down("ctr1"));
        let mut answered = runtimes(&["vm1", "ctr1", "vm2", "ctr2"]);
        answered.domains = None;
        let states =
            follow_started_links(&host, &lab, &ids(&["ctr1"]), &answered, &link_states).await;
        assert_eq!(states, HashMap::new());
        assert_eq!(host.take_carrier_writes(), vec![]);
        assert_eq!(link_states.of_lab("lab")[&0], peer_down("ctr1"));
    }

    #[tokio::test]
    async fn an_end_a_user_took_down_stays_down_when_the_state_is_the_same() {
        // A resume of a node that was already running finds the link attached, as recorded.
        let (host, lab) = lab();
        let link_states = LinkStates::default();
        link_states.record("lab", 0, LinkState::Attached);
        host.set_carrier_by_hand("tpb0", false);
        follow_started_links(
            &host,
            &lab,
            &ids(&["ctr1"]),
            &runtimes(&["vm1", "ctr1", "vm2", "ctr2"]),
            &link_states,
        )
        .await;
        assert_eq!(host.take_carrier_writes(), vec![]);
        assert!(!host.has_carrier("tpb0"));
    }
}
