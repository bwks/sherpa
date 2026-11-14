//! The carrier of a P2p link's ends follows the link's state, so a node sees its link go down
//! when its neighbour stops, as it would on a cable.
//!
//! - `Attached`: both ends have carrier.
//! - `PeerDown`: the end whose peer is down has no carrier. The peer's own end is left alone:
//!   it is gone, or its node is paused. When both nodes are down, the state names the first,
//!   and the second's end is taken down.
//! - `Failed`: neither end has carrier, so a node sees a dead link rather than a black hole.
//!
//! A container's end is its host veth, set down; the container sees its interface lose
//! carrier. A VM or unikernel end is the interface's link state in the live domain: a host tap
//! set down is not seen by the guest. The eBPF redirect is not touched.
//!
//! Carrier is changed only when a link's state changes, so a user who takes an end down by hand
//! to simulate a failure keeps it down while the state stays the same. A link's state is
//! changed when the carrier its new state wants for an end differs from what the state last
//! recorded for it (`LinkStates`) wants. Two cases set the carrier without a change:
//!
//! - nothing is recorded for the link: the first scan after sherpad starts, or a link no scan,
//!   resume or redeploy has recorded yet;
//! - the end was made again (a cold boot, a container start, a redeploy, a repair), as a new end
//!   always starts with carrier, whatever was set on the old one.
//!
//! Each end's carrier is read first and set only when it differs, so these never flap an end
//! that already has the carrier its link wants. The cost of the first case: a link taken down
//! by hand is brought back up by the first scan after sherpad restarts.

use std::collections::{HashMap, HashSet};

use tracing::instrument;

use super::host::LinkHost;
use super::reattach::LabLinks;

use shared::data::{DbLink, LinkState, RecordId};

/// The carrier link state `state` wants on each end, A then B: `Some(up)`, or `None` to leave
/// the end alone. `names` holds the nodes of the two ends.
fn wanted(state: &LinkState, names: [&str; 2]) -> [Option<bool>; 2] {
    match state {
        LinkState::Attached => [Some(true); 2],
        LinkState::Failed { .. } => [Some(false); 2],
        LinkState::PeerDown { peer } => {
            let survivor = |own: &str, other: &str| (own != peer && other == peer).then_some(false);
            [survivor(names[0], names[1]), survivor(names[1], names[0])]
        }
    }
}

/// The carrier to set on each end of a link that went from `previous` to `state`, A then B:
/// `Some(up)` to set, `None` to leave. An end is set when the carrier wanted for it changed,
/// when there is no previous state, or when it is `fresh`, made again.
pub(crate) fn carrier_moves(
    previous: Option<&LinkState>,
    state: &LinkState,
    names: [&str; 2],
    fresh: [bool; 2],
) -> [Option<bool>; 2] {
    let now = wanted(state, names);
    let before = previous.map(|previous| wanted(previous, names));
    let mut moves = [None; 2];
    for end in 0..2 {
        let changed = before.is_none_or(|before| before[end] != now[end]);
        if changed || fresh[end] {
            moves[end] = now[end];
        }
    }
    moves
}

/// Make the carrier of the ends of each link in `states` follow its state, from what
/// `previous` holds of it. `fresh` holds the host interfaces of ends made again since the
/// states were last recorded.
///
/// An end that does not exist is left; one whose carrier cannot be read or set is logged and
/// left, and the other ends are still set.
#[instrument(skip_all, fields(lab_id = %lab.lab_id()), level = "debug")]
pub(crate) async fn follow_carrier(
    host: &impl LinkHost,
    lab: &LabLinks,
    previous: &HashMap<u16, LinkState>,
    states: &HashMap<u16, LinkState>,
    fresh: &HashSet<String>,
) {
    let mut links: Vec<&DbLink> = lab
        .links()
        .iter()
        .filter(|link| states.contains_key(&link.index))
        .collect();
    links.sort_by_key(|link| link.index);
    for link in links {
        let Some(state) = states.get(&link.index) else {
            continue;
        };
        let name_a = lab.name(&link.node_a);
        let name_b = lab.name(&link.node_b);
        let moves = carrier_moves(
            previous.get(&link.index),
            state,
            [&name_a, &name_b],
            [fresh.contains(&link.tap_a), fresh.contains(&link.tap_b)],
        );
        let ends = [(&link.node_a, &link.tap_a), (&link.node_b, &link.tap_b)];
        for ((node, iface), up) in ends.into_iter().zip(moves) {
            if let Some(up) = up {
                set_end_carrier(host, lab, link, node, iface, up).await;
            }
        }
    }
}

/// Give end `iface` of `node` on `link` carrier, or take it, unless it already has what is
/// wanted or does not exist.
async fn set_end_carrier(
    host: &impl LinkHost,
    lab: &LabLinks,
    link: &DbLink,
    node: &RecordId,
    iface: &str,
    up: bool,
) {
    let device = lab.end_device(node, iface);
    let label = lab.label(link);
    match host.carrier(&device).await {
        Ok(Some(current)) if current == up => {}
        Ok(Some(_)) => match host.set_carrier(&device, up).await {
            Ok(()) => {
                tracing::info!(link = %label, %iface, up, "Set the carrier of a P2p link end");
            }
            Err(error) => {
                tracing::warn!(link = %label, %iface, up, error = %format!("{error:#}"), "Failed to set the carrier of a P2p link end");
            }
        },
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(link = %label, %iface, error = %format!("{error:#}"), "Failed to read the carrier of a P2p link end");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::fake::FakeHost;
    use super::super::host::EndDevice;
    use super::super::repair::check_lab_links;
    use shared::data::{BridgeKind, DbNode, LinkImpairment, NodeKind, NodeState};

    fn id(name: &str) -> RecordId {
        RecordId::new("node", name)
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

    /// vm1 linked to vm2 (0) and to ctr1 (1), every end made and wired, with carrier.
    fn lab() -> (FakeHost, LabLinks) {
        let nodes = ["vm1", "vm2", "ctr1"].map(node);
        let kinds = HashMap::from([
            (id("vm1"), NodeKind::VirtualMachine),
            (id("vm2"), NodeKind::VirtualMachine),
            (id("ctr1"), NodeKind::Container),
        ]);
        let lab = LabLinks::new(
            "lab",
            vec![link(0, "vm1", "vm2"), link(1, "vm1", "ctr1")],
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

    fn peer_down(peer: &str) -> LinkState {
        LinkState::PeerDown {
            peer: peer.to_string(),
        }
    }

    fn failed(reason: &str) -> LinkState {
        LinkState::Failed {
            reason: reason.to_string(),
        }
    }

    fn vm_tap(domain: &str, tap: &str) -> EndDevice {
        EndDevice::Tap {
            domain: domain.to_string(),
            tap: tap.to_string(),
        }
    }

    fn veth(iface: &str) -> EndDevice {
        EndDevice::Veth(iface.to_string())
    }

    /// Follow link 1 (vm1 <-> ctr1) from `previous` to `state`, with no end made again.
    async fn follow_link_1(
        host: &FakeHost,
        lab: &LabLinks,
        previous: Option<LinkState>,
        state: LinkState,
    ) -> Vec<(EndDevice, bool)> {
        let previous: HashMap<u16, LinkState> = previous.map(|p| (1, p)).into_iter().collect();
        let states = HashMap::from([(1, state)]);
        follow_carrier(host, lab, &previous, &states, &HashSet::new()).await;
        host.take_carrier_writes()
    }

    #[test]
    fn each_state_wants_the_carrier_of_its_ends() {
        let names = ["vm1", "ctr1"];
        assert_eq!(wanted(&LinkState::Attached, names), [Some(true); 2]);
        assert_eq!(wanted(&failed("x"), names), [Some(false); 2]);
        assert_eq!(wanted(&peer_down("ctr1"), names), [Some(false), None]);
        assert_eq!(wanted(&peer_down("vm1"), names), [None, Some(false)]);
        // A link between two interfaces of one node has no surviving end.
        assert_eq!(wanted(&peer_down("vm1"), ["vm1", "vm1"]), [None, None]);
    }

    #[test]
    fn a_state_that_does_not_change_moves_no_carrier() {
        let names = ["vm1", "ctr1"];
        for state in [
            LinkState::Attached,
            peer_down("ctr1"),
            peer_down("vm1"),
            failed("x"),
        ] {
            assert_eq!(
                carrier_moves(Some(&state), &state, names, [false; 2]),
                [None, None],
                "{state:?}"
            );
        }
        // A failure whose reason changed wants the same carrier.
        assert_eq!(
            carrier_moves(Some(&failed("a")), &failed("b"), names, [false; 2]),
            [None, None]
        );
    }

    #[test]
    fn only_the_ends_whose_wanted_carrier_changed_move() {
        let names = ["vm1", "ctr1"];
        assert_eq!(
            carrier_moves(
                Some(&LinkState::Attached),
                &peer_down("ctr1"),
                names,
                [false; 2]
            ),
            [Some(false), None]
        );
        assert_eq!(
            carrier_moves(
                Some(&peer_down("ctr1")),
                &LinkState::Attached,
                names,
                [false; 2]
            ),
            [Some(true), Some(true)]
        );
        assert_eq!(
            carrier_moves(Some(&peer_down("ctr1")), &failed("x"), names, [false; 2]),
            [None, Some(false)]
        );
        assert_eq!(
            carrier_moves(
                Some(&peer_down("vm1")),
                &peer_down("ctr1"),
                names,
                [false; 2]
            ),
            [Some(false), None],
            "vm1 came back while ctr1 stayed down: vm1's end goes down"
        );
    }

    #[test]
    fn an_end_made_again_or_a_link_with_no_state_gets_what_its_state_wants() {
        let names = ["vm1", "vm2"];
        assert_eq!(
            carrier_moves(
                Some(&peer_down("vm1")),
                &peer_down("vm1"),
                names,
                [false, true]
            ),
            [None, Some(false)]
        );
        assert_eq!(
            carrier_moves(None, &LinkState::Attached, names, [false; 2]),
            [Some(true), Some(true)]
        );
        assert_eq!(
            carrier_moves(None, &peer_down("vm2"), names, [false; 2]),
            [Some(false), None]
        );
    }

    #[tokio::test]
    async fn a_container_that_stops_takes_its_vm_peers_link_down_through_libvirt() {
        let (host, lab) = lab();
        host.remove(&["tpb1"]);
        let writes = follow_link_1(&host, &lab, Some(LinkState::Attached), peer_down("ctr1")).await;
        assert_eq!(writes, vec![(vm_tap("vm1-lab", "tpa1"), false)]);
        assert!(!host.has_carrier("tpa1"));

        // The container starts and its link is attached again: the VM's link comes back.
        host.make(&["tpb1"]);
        let writes = follow_link_1(&host, &lab, Some(peer_down("ctr1")), LinkState::Attached).await;
        assert_eq!(writes, vec![(vm_tap("vm1-lab", "tpa1"), true)]);
        assert!(host.has_carrier("tpa1") && host.has_carrier("tpb1"));
    }

    #[tokio::test]
    async fn a_vm_that_stops_takes_its_container_peers_host_veth_down() {
        let (host, lab) = lab();
        host.remove(&["tpa1"]);
        let writes = follow_link_1(&host, &lab, Some(LinkState::Attached), peer_down("vm1")).await;
        assert_eq!(writes, vec![(veth("tpb1"), false)]);
        assert!(!host.has_carrier("tpb1"));

        host.make(&["tpa1"]);
        let writes = follow_link_1(&host, &lab, Some(peer_down("vm1")), LinkState::Attached).await;
        assert_eq!(
            writes,
            vec![(veth("tpb1"), true)],
            "the VM's new tap already has carrier"
        );
    }

    #[tokio::test]
    async fn a_failed_link_takes_both_ends_down_and_an_attach_brings_them_up() {
        let (host, lab) = lab();
        let writes = follow_link_1(&host, &lab, Some(LinkState::Attached), failed("x")).await;
        assert_eq!(
            writes,
            vec![(vm_tap("vm1-lab", "tpa1"), false), (veth("tpb1"), false)]
        );
        let writes = follow_link_1(&host, &lab, Some(failed("x")), LinkState::Attached).await;
        assert_eq!(
            writes,
            vec![(vm_tap("vm1-lab", "tpa1"), true), (veth("tpb1"), true)]
        );
    }

    #[tokio::test]
    async fn an_unchanged_state_keeps_an_end_a_user_took_down() {
        // `ip link set tpb1 down` to simulate a failure: the scans that follow find the link
        // attached, as it was, and leave the end down.
        let (host, lab) = lab();
        host.set_carrier_by_hand("tpb1", false);
        for _ in 0..3 {
            let writes =
                follow_link_1(&host, &lab, Some(LinkState::Attached), LinkState::Attached).await;
            assert_eq!(writes, vec![]);
        }
        assert!(!host.has_carrier("tpb1"));
    }

    #[tokio::test]
    async fn a_paused_peer_counts_as_down() {
        // `sherpa down` or `docker pause`: the container's veth is still there, but the node is
        // not running, so the scan finds its link's peer down and the VM's link goes down. The
        // paused container's own end is left alone.
        let (host, lab) = lab();
        let previous = HashMap::from([(0, LinkState::Attached), (1, LinkState::Attached)]);
        let running = HashSet::from([id("vm1"), id("vm2")]);
        let check = check_lab_links(&host, &lab, &running).await;
        let fresh = check.fresh_ends();
        let states = check.into_states();
        assert_eq!(states[&1], peer_down("ctr1"));
        follow_carrier(&host, &lab, &previous, &states, &fresh).await;
        assert_eq!(
            host.take_carrier_writes(),
            vec![(vm_tap("vm1-lab", "tpa1"), false)]
        );
        assert!(host.has_carrier("tpb1"));

        // Unpaused: the link is attached as it was, and the VM's link comes back.
        let running = HashSet::from([id("vm1"), id("vm2"), id("ctr1")]);
        let check = check_lab_links(&host, &lab, &running).await;
        let fresh = check.fresh_ends();
        let next = check.into_states();
        assert_eq!(next[&1], LinkState::Attached);
        follow_carrier(&host, &lab, &states, &next, &fresh).await;
        assert_eq!(
            host.take_carrier_writes(),
            vec![(vm_tap("vm1-lab", "tpa1"), true)]
        );
    }

    #[tokio::test]
    async fn both_peers_down_take_the_end_of_the_second_down() {
        // `sherpa down` shuts vm1 off and pauses ctr1: the state names vm1, and ctr1's paused
        // end is taken down, so it has no carrier when ctr1 is unpaused before vm1 is back.
        let (host, lab) = lab();
        host.remove(&["tpa0", "tpa1"]);
        let writes = follow_link_1(&host, &lab, Some(LinkState::Attached), peer_down("vm1")).await;
        assert_eq!(writes, vec![(veth("tpb1"), false)]);
        // ctr1 unpaused, vm1 still off: the state is the same and the end stays down.
        let writes = follow_link_1(&host, &lab, Some(peer_down("vm1")), peer_down("vm1")).await;
        assert_eq!(writes, vec![]);
        assert!(!host.has_carrier("tpb1"));
    }

    #[tokio::test]
    async fn the_first_scan_after_a_restart_sets_only_the_carrier_that_is_wrong() {
        // sherpad restarted: nothing is recorded. Link 0's peer vm2 stopped while sherpad was
        // down; link 1 was taken down before the restart and is still down.
        let (host, lab) = lab();
        host.remove(&["tpb0", "tpb1"]);
        host.set_carrier_by_hand("tpa1", false);
        let running = HashSet::from([id("vm1")]);
        let check = check_lab_links(&host, &lab, &running).await;
        let fresh = check.fresh_ends();
        let states = check.into_states();
        follow_carrier(&host, &lab, &HashMap::new(), &states, &fresh).await;
        assert_eq!(
            host.take_carrier_writes(),
            vec![(vm_tap("vm1-lab", "tpa0"), false)],
            "tpa1 already has no carrier: it is not set again"
        );

        // The second scan has the first's states: nothing changed, nothing is set.
        let check = check_lab_links(&host, &lab, &running).await;
        let fresh = check.fresh_ends();
        let next = check.into_states();
        follow_carrier(&host, &lab, &states, &next, &fresh).await;
        assert_eq!(host.take_carrier_writes(), vec![]);
    }

    #[tokio::test]
    async fn the_first_scan_after_a_restart_does_not_flap_a_working_lab() {
        let (host, lab) = lab();
        let running = HashSet::from([id("vm1"), id("vm2"), id("ctr1")]);
        let check = check_lab_links(&host, &lab, &running).await;
        let fresh = check.fresh_ends();
        let states = check.into_states();
        follow_carrier(&host, &lab, &HashMap::new(), &states, &fresh).await;
        assert_eq!(host.take_carrier_writes(), vec![]);
    }

    #[tokio::test]
    async fn a_vm_started_while_its_peer_is_down_has_its_new_end_taken_down() {
        // vm1 and vm2 were both off; vm2 is started outside Sherpa. Its new tap is bare, and the
        // link's state names vm1 as before, but the new tap starts with carrier.
        let (host, lab) = lab();
        host.remove(&["tpa0", "tpa1"]);
        host.make(&["tpb0"]);
        let previous = HashMap::from([(0, peer_down("vm1"))]);
        let running = HashSet::from([id("vm2"), id("ctr1")]);
        let check = check_lab_links(&host, &lab, &running).await;
        let fresh = check.fresh_ends();
        let states = check.into_states();
        assert_eq!(states[&0], peer_down("vm1"));
        follow_carrier(&host, &lab, &previous, &states, &fresh).await;
        assert!(
            host.take_carrier_writes()
                .contains(&(vm_tap("vm2-lab", "tpb0"), false))
        );
    }

    #[tokio::test]
    async fn an_end_that_does_not_exist_or_fails_does_not_stop_the_others() {
        let (mut host, lab) = lab();
        host.remove(&["tpa0"]);
        host.failing_carrier.insert("tpa1".to_string());
        let previous = HashMap::from([(0, LinkState::Attached), (1, LinkState::Attached)]);
        let states = HashMap::from([(0, failed("x")), (1, failed("y"))]);
        follow_carrier(&host, &lab, &previous, &states, &HashSet::new()).await;
        assert_eq!(
            host.take_carrier_writes(),
            vec![(vm_tap("vm2-lab", "tpb0"), false), (veth("tpb1"), false)]
        );
    }
}
