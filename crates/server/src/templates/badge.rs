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

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    const SIZES: [BadgeSize; 2] = [BadgeSize::Small, BadgeSize::Large];

    #[test]
    fn every_lab_phase_has_a_label_and_a_tone() {
        for (phase, label, tone) in [
            (LabPhase::Creating, "Creating", BadgeTone::Warning),
            (LabPhase::Ready, "Ready", BadgeTone::Neutral),
            (LabPhase::Destroying, "Destroying", BadgeTone::Warning),
            (LabPhase::DestroyFailed, "Destroy failed", BadgeTone::Danger),
            (LabPhase::Interrupted, "Interrupted", BadgeTone::Danger),
            (LabPhase::Unknown, "Unknown", BadgeTone::Danger),
        ] {
            assert_eq!(BadgeTone::for_phase(phase), tone, "{phase:?}");
            for size in SIZES {
                assert_eq!(
                    Badge::for_phase(phase, size),
                    Badge { label, tone, size },
                    "{phase:?} {size:?}"
                );
            }
        }
    }

    const LAB_STATES: [(LabState, &str, BadgeTone); 7] = [
        (LabState::Running, "Running", BadgeTone::Success),
        (LabState::Stopped, "Stopped", BadgeTone::Neutral),
        (LabState::Partial, "Partial", BadgeTone::Warning),
        (LabState::Starting, "Starting", BadgeTone::Warning),
        (LabState::Failed, "Failed", BadgeTone::Danger),
        (LabState::Empty, "Empty", BadgeTone::Neutral),
        (LabState::Unknown, "Unknown", BadgeTone::Neutral),
    ];

    #[test]
    fn the_table_covers_every_lab_state() {
        assert_eq!(LAB_STATES.len(), LabState::iter().count());
    }

    #[test]
    fn a_ready_lab_shows_its_state() {
        for (state, label, tone) in LAB_STATES {
            assert_eq!(BadgeTone::for_lab_state(state), tone, "{state:?}");
            for size in SIZES {
                assert_eq!(
                    Badge::for_lab(LabPhase::Ready, state, size),
                    Badge { label, tone, size },
                    "{state:?} {size:?}"
                );
            }
        }
    }

    #[test]
    fn a_lab_that_is_not_ready_shows_its_phase_whatever_its_state() {
        for phase in LabPhase::iter().filter(|phase| !phase.is_ready()) {
            for (state, _, _) in LAB_STATES {
                for size in SIZES {
                    assert_eq!(
                        Badge::for_lab(phase, state, size),
                        Badge::for_phase(phase, size),
                        "{phase:?} {state:?} {size:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_node_state_has_a_label_and_a_tone() {
        let states = [
            (NodeState::Building, "Building", BadgeTone::Warning),
            (NodeState::Starting, "Starting", BadgeTone::Warning),
            (NodeState::Running, "Running", BadgeTone::Success),
            (NodeState::Stopped, "Stopped", BadgeTone::Neutral),
            (NodeState::Failed, "Failed", BadgeTone::Danger),
            (NodeState::Missing, "Missing", BadgeTone::Danger),
            (NodeState::Unknown, "Unknown", BadgeTone::Neutral),
        ];
        assert_eq!(states.len(), NodeState::iter().count());
        for (state, label, tone) in states {
            assert_eq!(BadgeTone::for_node_state(state), tone, "{state:?}");
            for size in SIZES {
                assert_eq!(
                    Badge::for_node_state(state, size),
                    Badge { label, tone, size },
                    "{state:?} {size:?}"
                );
            }
        }
    }
}
