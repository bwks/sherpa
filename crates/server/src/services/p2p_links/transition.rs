//! The state machine of one P2p link when one of its ends is made again.
//!
//! A link's state is held by the kernel: whether each end's interface exists, and which ifindex
//! each end's eBPF redirect points at. Making an end again (a cold boot, a container start, a
//! redeploy) gives it a new ifindex, so the link is stale until it is re-attached. Re-attaching
//! looks at both ends and takes one step:
//!
//! - both ends exist: point each end's redirect at the other and apply the impairments, which
//!   leaves the link [`LinkState::Attached`], or [`LinkState::Failed`] if the kernel refuses;
//! - the peer's end does not exist: [`LinkState::PeerDown`]. Nothing is touched. The peer's
//!   own re-attach, when it starts, finds both ends and attaches both directions;
//! - the node's own end does not exist, or an end could not be made or looked up:
//!   [`LinkState::Failed`].

use shared::data::LinkState;

/// One end of a link, as the kernel shows it when the link is re-attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum End {
    /// The interface exists, with this index.
    Present(u32),
    /// There is no interface: its node is down, or the end was never made.
    Absent,
    /// The end could not be made or looked up, and why.
    Broken(String),
}

/// Which ends of a link belong to the node being re-attached. A link may join two interfaces
/// of one node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    A,
    B,
    Both,
}

/// One end of a link: its host interface and what the kernel shows of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EndAt<'a> {
    pub(crate) iface: &'a str,
    pub(crate) end: End,
}

/// What re-attaching a link does, decided from its two ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LinkStep {
    /// Point each end's redirect at the other end's index, and apply the impairments.
    Attach { ifindex_a: u32, ifindex_b: u32 },
    /// Change nothing in the kernel, and leave the link in this state.
    Leave(LinkState),
}

/// The step that re-attaches a link whose `side` ends belong to the node that was made again,
/// with ends `a` and `b`. `peer` names the other node, for a peer that is down.
///
/// When both ends are wrong, the node's own end is the one reported: a link whose own end is
/// missing has failed, whatever the peer's state.
pub(crate) fn step(side: Side, a: &EndAt<'_>, b: &EndAt<'_>, peer: &str) -> LinkStep {
    let own_a = side != Side::B;
    let own_b = side != Side::A;
    match (resolve(a, own_a, peer), resolve(b, own_b, peer)) {
        (Ok(ifindex_a), Ok(ifindex_b)) => LinkStep::Attach {
            ifindex_a,
            ifindex_b,
        },
        (Err(state), Ok(_)) | (Ok(_), Err(state)) => LinkStep::Leave(state),
        (Err(state_a), Err(state_b)) => LinkStep::Leave(if own_a { state_a } else { state_b }),
    }
}

/// The index of `end`, or the state its absence leaves the link in. A missing end is the peer
/// being down, unless it is the node's `own` end.
fn resolve(end: &EndAt<'_>, own: bool, peer: &str) -> Result<u32, LinkState> {
    match &end.end {
        End::Present(ifindex) => Ok(*ifindex),
        End::Absent if own => Err(LinkState::Failed {
            reason: format!("its end {} does not exist", end.iface),
        }),
        End::Absent => Err(LinkState::PeerDown {
            peer: peer.to_string(),
        }),
        End::Broken(reason) => Err(LinkState::Failed {
            reason: reason.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(iface: &str, end: End) -> EndAt<'_> {
        EndAt { iface, end }
    }

    #[test]
    fn a_link_with_both_ends_is_attached_both_ways() {
        let step = step(
            Side::A,
            &at("tpa0", End::Present(10)),
            &at("tpb0", End::Present(20)),
            "vm2",
        );
        assert_eq!(
            step,
            LinkStep::Attach {
                ifindex_a: 10,
                ifindex_b: 20
            }
        );
    }

    #[test]
    fn a_link_whose_peer_end_is_missing_waits_for_its_peer() {
        // M9.2 and M25.1: a peer that is shut off has no tap. That is expected, not a failure.
        for side in [Side::A, Side::B] {
            let (a, b) = match side {
                Side::A => (End::Present(10), End::Absent),
                _ => (End::Absent, End::Present(10)),
            };
            assert_eq!(
                step(side, &at("tpa0", a), &at("tpb0", b), "vm2"),
                LinkStep::Leave(LinkState::PeerDown {
                    peer: "vm2".to_string()
                }),
                "{side:?}"
            );
        }
    }

    #[test]
    fn a_link_whose_own_end_is_missing_has_failed_even_with_its_peer_down() {
        assert_eq!(
            step(
                Side::B,
                &at("tpa0", End::Absent),
                &at("tpb0", End::Absent),
                "vm1"
            ),
            LinkStep::Leave(LinkState::Failed {
                reason: "its end tpb0 does not exist".to_string()
            })
        );
    }

    #[test]
    fn an_end_that_could_not_be_made_or_seen_fails_the_link() {
        let broken = || End::Broken("netlink refused".to_string());
        for (side, a, b) in [
            (Side::A, broken(), End::Present(2)),
            (Side::A, End::Present(1), broken()),
            (Side::B, End::Present(1), broken()),
        ] {
            assert_eq!(
                step(side, &at("tpa0", a), &at("tpb0", b), "peer"),
                LinkStep::Leave(LinkState::Failed {
                    reason: "netlink refused".to_string()
                })
            );
        }
    }

    #[test]
    fn a_link_between_two_interfaces_of_one_node_needs_both() {
        assert_eq!(
            step(
                Side::Both,
                &at("tpa0", End::Present(1)),
                &at("tpb0", End::Absent),
                "vm1"
            ),
            LinkStep::Leave(LinkState::Failed {
                reason: "its end tpb0 does not exist".to_string()
            })
        );
        assert_eq!(
            step(
                Side::Both,
                &at("tpa0", End::Present(1)),
                &at("tpb0", End::Present(2)),
                "vm1"
            ),
            LinkStep::Attach {
                ifindex_a: 1,
                ifindex_b: 2
            }
        );
    }
}
