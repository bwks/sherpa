//! Which actions the web UI offers for a lab or a node. Rust decides them from the lab's phase
//! and state or the node's state; the templates only draw the buttons they are given.

use shared::data::{LabPhase, LabState, NodeState};

/// The start and stop buttons a lab view shows. Destroy is always offered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LabActions {
    pub start: bool,
    pub stop: bool,
    pub resume_with_cli: bool,
}

/// The buttons a node view shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NodeActions {
    pub start: bool,
    pub stop: bool,
    pub redeploy: bool,
}

impl LabActions {
    /// The lab's actions, with `sherpa resume` in place of start for a lab with a Tailscale gateway.
    pub fn for_lab(phase: LabPhase, state: LabState, has_gateway: bool) -> Self {
        let actions = Self::for_phase(phase, state);
        Self {
            start: actions.start && !has_gateway,
            resume_with_cli: actions.start && has_gateway,
            ..actions
        }
    }

    /// Only a ready lab can be started or stopped; any other can only be destroyed. A ready lab
    /// offers stop while any node runs or starts, and start while any node is not running.
    fn for_phase(phase: LabPhase, state: LabState) -> Self {
        match phase {
            LabPhase::Ready => match state {
                LabState::Running | LabState::Starting => Self {
                    start: false,
                    stop: true,
                    resume_with_cli: false,
                },
                LabState::Partial => Self {
                    start: true,
                    stop: true,
                    resume_with_cli: false,
                },
                LabState::Stopped | LabState::Failed | LabState::Empty | LabState::Unknown => {
                    Self {
                        start: true,
                        stop: false,
                        resume_with_cli: false,
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
    /// The node's actions for its state, with redeploy offered only to the lab's owner.
    pub fn for_node(state: NodeState, owns_lab: bool) -> Self {
        let actions = Self::for_state(state);
        Self {
            redeploy: actions.redeploy && owns_lab,
            ..actions
        }
    }

    /// A node being built offers nothing; a missing one only redeploy.
    fn for_state(state: NodeState) -> Self {
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
    use super::*;

    #[test]
    fn lab_page_016_an_admin_who_does_not_own_the_lab_is_offered_no_redeploy() {
        for state in [
            NodeState::Building,
            NodeState::Starting,
            NodeState::Running,
            NodeState::Stopped,
            NodeState::Failed,
            NodeState::Missing,
            NodeState::Unknown,
        ] {
            assert_eq!(
                NodeActions::for_node(state, false),
                NodeActions {
                    redeploy: false,
                    ..NodeActions::for_node(state, true)
                },
                "{state}"
            );
        }
    }
}
