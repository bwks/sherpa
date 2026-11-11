//! Which actions the web UI offers for a lab or a node. Rust decides them from the lab's phase
//! and state or the node's state; the templates only draw the buttons they are given.

use shared::data::{LabPhase, LabState, NodeState};

/// The start and stop buttons a lab view shows. Destroy is always offered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LabActions {
    pub start: bool,
    pub stop: bool,
}

/// The buttons a node view shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NodeActions {
    pub start: bool,
    pub stop: bool,
    pub redeploy: bool,
}

impl LabActions {
    /// Only a ready lab can be started or stopped; any other can only be destroyed. A ready lab
    /// offers stop while any node runs or starts, and start while any node is not running.
    pub fn for_lab(phase: LabPhase, state: LabState) -> Self {
        match phase {
            LabPhase::Ready => match state {
                LabState::Running | LabState::Starting => Self {
                    start: false,
                    stop: true,
                },
                LabState::Partial => Self {
                    start: true,
                    stop: true,
                },
                LabState::Stopped | LabState::Failed | LabState::Empty | LabState::Unknown => {
                    Self {
                        start: true,
                        stop: false,
                    }
                }
            },
            LabPhase::Creating
            | LabPhase::Destroying
            | LabPhase::DestroyFailed
            | LabPhase::Interrupted
            | LabPhase::Unknown => Self::default(),
        }
    }
}

impl NodeActions {
    /// A node being built offers nothing; a missing one only redeploy.
    pub fn for_state(state: NodeState) -> Self {
        match state {
            NodeState::Building => Self::default(),
            NodeState::Starting | NodeState::Running => Self {
                start: false,
                stop: true,
                redeploy: true,
            },
            NodeState::Stopped | NodeState::Failed | NodeState::Unknown => Self {
                start: true,
                stop: false,
                redeploy: true,
            },
            NodeState::Missing => Self {
                start: false,
                stop: false,
                redeploy: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use shared::data::OperationEvent;
    use strum::IntoEnumIterator;

    use super::*;

    const START: LabActions = LabActions {
        start: true,
        stop: false,
    };
    const STOP: LabActions = LabActions {
        start: false,
        stop: true,
    };
    const BOTH: LabActions = LabActions {
        start: true,
        stop: true,
    };

    #[test]
    fn a_ready_lab_offers_the_actions_its_state_allows() {
        let cases = [
            (LabState::Running, STOP),
            (LabState::Stopped, START),
            (LabState::Partial, BOTH),
            (LabState::Starting, STOP),
            (LabState::Failed, START),
            (LabState::Empty, START),
            (LabState::Unknown, START),
        ];
        assert_eq!(cases.len(), LabState::iter().count());
        for (state, actions) in cases {
            assert_eq!(
                LabActions::for_lab(LabPhase::Ready, state),
                actions,
                "{state:?}"
            );
        }
    }

    #[test]
    fn a_lab_that_is_not_ready_offers_no_start_or_stop() {
        for phase in LabPhase::iter().filter(|phase| !phase.is_ready()) {
            for state in LabState::iter() {
                assert_eq!(
                    LabActions::for_lab(phase, state),
                    LabActions::default(),
                    "{phase:?} {state:?}"
                );
            }
        }
    }

    #[test]
    fn a_node_offers_the_actions_its_state_allows() {
        let none = NodeActions::default();
        let stop = NodeActions {
            stop: true,
            redeploy: true,
            ..none
        };
        let start = NodeActions {
            start: true,
            redeploy: true,
            ..none
        };
        let redeploy = NodeActions {
            redeploy: true,
            ..none
        };
        let cases = [
            (NodeState::Building, none),
            (NodeState::Starting, stop),
            (NodeState::Running, stop),
            (NodeState::Stopped, start),
            (NodeState::Failed, start),
            (NodeState::Missing, redeploy),
            (NodeState::Unknown, start),
        ];
        assert_eq!(cases.len(), NodeState::iter().count());
        for (state, actions) in cases {
            assert_eq!(NodeActions::for_state(state), actions, "{state:?}");
        }
    }

    #[test]
    fn a_node_offers_only_actions_its_lifecycle_allows() {
        for state in NodeState::iter() {
            let actions = NodeActions::for_state(state);
            for (offered, event) in [
                (actions.start, OperationEvent::Launched),
                (actions.stop, OperationEvent::Stopped),
                (actions.redeploy, OperationEvent::RebuildStarted),
            ] {
                if offered {
                    assert!(state.on_operation(event).is_ok(), "{state:?} {event:?}");
                }
            }
        }
    }
}
