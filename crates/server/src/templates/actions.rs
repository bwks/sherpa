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
