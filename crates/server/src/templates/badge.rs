//! Status badges for the web UI. Rust decides each badge's label and tone; the
//! `snippets/badge.html.jinja` partial only maps them to the `badge-*` classes.

use shared::data::LabPhase;

/// The colour a badge is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeTone {
    Neutral,
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
}

/// What `snippets/badge.html.jinja` renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Badge {
    pub label: &'static str,
    pub tone: BadgeTone,
    pub size: BadgeSize,
}

impl Badge {
    /// A lab phase badge for a table row.
    pub fn phase_small(phase: LabPhase) -> Self {
        Self::phase(phase, BadgeSize::Small)
    }

    /// A lab phase badge for a detail page header.
    pub fn phase_large(phase: LabPhase) -> Self {
        Self::phase(phase, BadgeSize::Large)
    }

    fn phase(phase: LabPhase, size: BadgeSize) -> Self {
        Self {
            label: phase_label(phase),
            tone: BadgeTone::for_phase(phase),
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

#[cfg(test)]
mod tests {
    use super::*;

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
            assert_eq!(
                Badge::phase_small(phase),
                Badge {
                    label,
                    tone,
                    size: BadgeSize::Small
                },
                "{phase:?}"
            );
            assert_eq!(
                Badge::phase_large(phase),
                Badge {
                    label,
                    tone,
                    size: BadgeSize::Large
                },
                "{phase:?}"
            );
        }
    }
}
