//! Status badges for the web UI. Rust decides each badge's label and tone; the
//! `snippets/badge.html.jinja` partial only maps them to the `badge-*` classes.

use shared::data::{LabPhase, LabState, NodeState};

/// The colour a badge is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeTone {
    Neutral,
    Success,
    Warning,
    Danger,
}

/// Small badges sit in table rows, large ones in detail page headers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeSize {
    Small,
    Large,
}

impl BadgeTone {
    /// An operation still owns the lab: a warning. Nothing will finish it: danger.
    pub fn for_phase(phase: LabPhase) -> Self {
        match phase {
            LabPhase::Ready => Self::Neutral,
            LabPhase::Creating | LabPhase::Destroying => Self::Warning,
            LabPhase::DestroyFailed | LabPhase::Interrupted | LabPhase::Unknown => Self::Danger,
        }
    }

    /// Running is a success, on the way there a warning, and a failure danger.
    pub fn for_lab_state(state: LabState) -> Self {
        match state {
            LabState::Running => Self::Success,
            LabState::Starting | LabState::Partial => Self::Warning,
            LabState::Failed => Self::Danger,
            LabState::Stopped | LabState::Empty | LabState::Unknown => Self::Neutral,
        }
    }

    /// Running is a success, on the way there a warning, and a fault or a lost node danger.
    pub fn for_node_state(state: NodeState) -> Self {
        match state {
            NodeState::Running => Self::Success,
            NodeState::Building | NodeState::Starting => Self::Warning,
            NodeState::Failed | NodeState::Missing => Self::Danger,
            NodeState::Stopped | NodeState::Unknown => Self::Neutral,
        }
    }
}

/// What `snippets/badge.html.jinja` renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Badge {
    pub label: &'static str,
    pub tone: BadgeTone,
    pub size: BadgeSize,
}

impl Badge {
    /// A lab phase badge drawn at `size`.
    pub fn for_phase(phase: LabPhase, size: BadgeSize) -> Self {
        Self {
            label: phase_label(phase),
            tone: BadgeTone::for_phase(phase),
            size,
        }
    }

    /// The badge a lab view shows for a lab: its phase until it is ready, then its state.
    pub fn for_lab(phase: LabPhase, state: LabState, size: BadgeSize) -> Self {
        match phase {
            LabPhase::Ready => Self {
                label: lab_state_label(state),
                tone: BadgeTone::for_lab_state(state),
                size,
            },
            LabPhase::Creating
            | LabPhase::Destroying
            | LabPhase::DestroyFailed
            | LabPhase::Interrupted
            | LabPhase::Unknown => Self::for_phase(phase, size),
        }
    }

    /// A node state badge drawn at `size`.
    pub fn for_node_state(state: NodeState, size: BadgeSize) -> Self {
        Self {
            label: node_state_label(state),
            tone: BadgeTone::for_node_state(state),
            size,
        }
    }
}

fn phase_label(phase: LabPhase) -> &'static str {
    match phase {
        LabPhase::Creating => "Creating",
        LabPhase::Ready => "Ready",
        LabPhase::Destroying => "Destroying",
        LabPhase::DestroyFailed => "Destroy failed",
        LabPhase::Interrupted => "Interrupted",
        LabPhase::Unknown => "Unknown",
    }
}

fn lab_state_label(state: LabState) -> &'static str {
    match state {
        LabState::Running => "Running",
        LabState::Stopped => "Stopped",
        LabState::Partial => "Partial",
        LabState::Starting => "Starting",
        LabState::Failed => "Failed",
        LabState::Empty => "Empty",
        LabState::Unknown => "Unknown",
    }
}

fn node_state_label(state: NodeState) -> &'static str {
    match state {
        NodeState::Building => "Building",
        NodeState::Starting => "Starting",
        NodeState::Running => "Running",
        NodeState::Stopped => "Stopped",
        NodeState::Failed => "Failed",
        NodeState::Missing => "Missing",
        NodeState::Unknown => "Unknown",
    }
}
