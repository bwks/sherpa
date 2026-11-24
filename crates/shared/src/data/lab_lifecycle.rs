//! The lab lifecycle: every allowed move between `LabPhase`s, in one place.
//!
//! The phase says which operation owns a lab's record: `up` while it is `Creating`, a destroy
//! while it is `Destroying`, none once it is `Ready`. Only a `Ready` lab takes `down`,
//! `resume` or a redeploy; a lab in any other phase can only be destroyed or cleaned.
//!
//! How healthy a ready lab is, `LabState`, is derived from its nodes when it is read. The
//! phase is not: a lab being created may have no nodes yet, and a lab whose destroy failed may
//! have nodes that look stopped.
//!
//! The server writes each move with a compare-and-set on the phase it read, while it holds the
//! lab's operation lease.

use std::fmt;

use schemars::JsonSchema;
use serde_derive::{Deserialize, Serialize};
use strum_macros::EnumIter;

/// Which operation, if any, owns a lab's record.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, EnumIter,
)]
#[serde(rename_all = "snake_case")]
pub enum LabPhase {
    Creating,
    #[default]
    Ready,
    Destroying,
    DestroyFailed,
    Interrupted,
    #[serde(other)]
    Unknown,
}

/// A step an operation took with a lab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter)]
pub enum LabEvent {
    Built,
    Released,
    DestroyStarted,
    DestroyFailed,
    Destroyed,
    OwnerLost,
}

/// What an allowed `LabEvent` does to the lab's record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabTransition {
    To(LabPhase),
    Removed,
}

/// A lab event the lab's phase does not allow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LabRefused {
    pub phase: LabPhase,
    pub event: LabEvent,
}

/// An operation on the lab's nodes (`down`, `resume`, redeploy) refused because the lab is not
/// `Ready`. Its message says what the user can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LabNotReady {
    pub phase: LabPhase,
}

impl LabPhase {
    /// What `event` does to a lab in this phase, or why it may not.
    ///
    /// # Errors
    /// `LabRefused` when the event is not allowed from this phase.
    pub fn on_event(self, event: LabEvent) -> Result<LabTransition, LabRefused> {
        let refused = Err(LabRefused { phase: self, event });
        match (self, event) {
            (LabPhase::Creating, LabEvent::Built) => Ok(LabTransition::To(LabPhase::Ready)),
            (LabPhase::Creating, LabEvent::Released)
            | (LabPhase::Destroying, LabEvent::Destroyed) => Ok(LabTransition::Removed),
            // A lab can always be destroyed, whatever state it was left in.
            (_, LabEvent::DestroyStarted) => Ok(LabTransition::To(LabPhase::Destroying)),
            (LabPhase::Destroying, LabEvent::DestroyFailed | LabEvent::OwnerLost) => {
                Ok(LabTransition::To(LabPhase::DestroyFailed))
            }
            (LabPhase::Creating, LabEvent::OwnerLost) => {
                Ok(LabTransition::To(LabPhase::Interrupted))
            }
            _ => refused,
        }
    }

    /// Whether operations on the lab's nodes may run: only once `up` has built it, and while
    /// no destroy has started.
    ///
    /// # Errors
    /// `LabNotReady` for any phase but `Ready`; its message says what to do.
    pub fn ensure_ready(self) -> Result<(), LabNotReady> {
        if self.is_ready() {
            Ok(())
        } else {
            Err(LabNotReady { phase: self })
        }
    }

    /// Whether the lab has been built and nothing is removing it. The web templates use it to
    /// choose between the node status and the phase.
    #[must_use]
    pub fn is_ready(self) -> bool {
        self == LabPhase::Ready
    }
}

impl fmt::Display for LabPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LabPhase::Creating => write!(f, "creating"),
            LabPhase::Ready => write!(f, "ready"),
            LabPhase::Destroying => write!(f, "destroying"),
            LabPhase::DestroyFailed => write!(f, "destroy failed"),
            LabPhase::Interrupted => write!(f, "interrupted"),
            LabPhase::Unknown => write!(f, "unknown"),
        }
    }
}

impl fmt::Display for LabEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LabEvent::Built => write!(f, "finish building"),
            LabEvent::Released => write!(f, "release"),
            LabEvent::DestroyStarted => write!(f, "start destroying"),
            LabEvent::DestroyFailed => write!(f, "fail destroying"),
            LabEvent::Destroyed => write!(f, "remove"),
            LabEvent::OwnerLost => write!(f, "recover"),
        }
    }
}

impl fmt::Display for LabRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot {} a lab that is {}", self.event, self.phase)
    }
}

impl std::error::Error for LabRefused {}

impl fmt::Display for LabNotReady {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.phase {
            LabPhase::Creating => write!(
                f,
                "the lab is being created; wait for `up` to finish, or destroy it"
            ),
            LabPhase::Ready => write!(f, "the lab is ready"),
            LabPhase::Destroying => write!(f, "the lab is being destroyed"),
            LabPhase::DestroyFailed => write!(
                f,
                "destroying the lab left resources behind; destroy it again"
            ),
            LabPhase::Interrupted => write!(
                f,
                "the server stopped while the lab was being created; destroy it and start it again"
            ),
            LabPhase::Unknown => write!(
                f,
                "the lab is in a phase this server does not know; destroy it"
            ),
        }
    }
}

impl std::error::Error for LabNotReady {}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    use LabPhase::{Creating, DestroyFailed, Destroying, Interrupted, Ready, Unknown};
    use LabTransition::{Removed, To};

    fn refused(phase: LabPhase, event: LabEvent) -> Result<LabTransition, LabRefused> {
        Err(LabRefused { phase, event })
    }

    #[test]
    fn lab_phase_001_a_lab_moves_only_as_allowed_and_a_refusal_names_the_move_and_phase() {
        assert_eq!(Creating.on_event(LabEvent::Built), Ok(To(Ready)));
        assert_eq!(Creating.on_event(LabEvent::Released), Ok(Removed));
        for phase in LabPhase::iter() {
            assert_eq!(
                phase.on_event(LabEvent::DestroyStarted),
                Ok(To(Destroying)),
                "{phase:?}"
            );
        }
        assert_eq!(Destroying.on_event(LabEvent::Destroyed), Ok(Removed));
        assert_eq!(
            Destroying.on_event(LabEvent::DestroyFailed),
            Ok(To(DestroyFailed))
        );

        // Once built, a failed second `up` has no reservation to give back.
        for event in [
            LabEvent::Built,
            LabEvent::Released,
            LabEvent::Destroyed,
            LabEvent::DestroyFailed,
            LabEvent::OwnerLost,
        ] {
            assert_eq!(Ready.on_event(event), refused(Ready, event), "{event:?}");
        }
        assert_eq!(
            refused(Ready, LabEvent::Built).unwrap_err().to_string(),
            "cannot finish building a lab that is ready"
        );
    }

    #[test]
    fn lab_phase_002_only_a_ready_lab_takes_operations_on_its_nodes() {
        for phase in LabPhase::iter() {
            assert_eq!(phase.ensure_ready().is_ok(), phase == Ready, "{phase:?}");
            assert_eq!(phase.is_ready(), phase == Ready, "{phase:?}");
        }
    }

    #[test]
    fn lab_phase_003_the_refusal_of_each_phase_says_what_to_do() {
        for (phase, refusal) in [
            (
                Creating,
                "the lab is being created; wait for `up` to finish, or destroy it",
            ),
            (
                DestroyFailed,
                "destroying the lab left resources behind; destroy it again",
            ),
            (
                Interrupted,
                "the server stopped while the lab was being created; destroy it and start it again",
            ),
            (
                Unknown,
                "the lab is in a phase this server does not know; destroy it",
            ),
        ] {
            assert_eq!(phase.ensure_ready().unwrap_err().to_string(), refusal);
        }
    }

    #[test]
    fn lab_phase_004_a_lab_that_lost_its_operation_is_marked_and_can_be_destroyed() {
        // The daemon stopped while `up` built the lab, or while a destroy removed it.
        assert_eq!(Creating.on_event(LabEvent::OwnerLost), Ok(To(Interrupted)));
        assert_eq!(
            Destroying.on_event(LabEvent::OwnerLost),
            Ok(To(DestroyFailed))
        );
        for phase in [Interrupted, DestroyFailed] {
            assert_eq!(phase.on_event(LabEvent::DestroyStarted), Ok(To(Destroying)));
        }
    }

    #[test]
    fn lab_phase_005_an_unknown_phase_reads_as_unknown_and_can_be_destroyed() {
        let phase: LabPhase = serde_json::from_str(r#""archived""#).unwrap();
        assert_eq!(phase, Unknown);
        assert!(phase.ensure_ready().is_err());
        assert_eq!(phase.on_event(LabEvent::DestroyStarted), Ok(To(Destroying)));
    }
}
