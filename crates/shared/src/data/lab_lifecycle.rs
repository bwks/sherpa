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
    /// `up` reserved the lab and is building it.
    Creating,
    /// `up` finished. Its nodes may be running, stopped or failed: see `LabState`.
    #[default]
    Ready,
    /// A destroy, a clean, or a failed `up`'s rollback is removing the lab.
    Destroying,
    /// A destroy left something behind. Destroying it again retries.
    DestroyFailed,
    /// The server stopped while `up` was building the lab. Nothing will finish it.
    Interrupted,
    /// A phase this build does not know, such as one written by a newer version.
    #[serde(other)]
    Unknown,
}

/// A step an operation took with a lab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter)]
pub enum LabEvent {
    /// `up` built the lab.
    Built,
    /// The `up` that reserved the lab failed before it created anything, and gives the
    /// reservation back.
    Released,
    /// A destroy, a clean, or a failed `up`'s rollback starts removing the lab.
    DestroyStarted,
    /// The destroy left something behind.
    DestroyFailed,
    /// The destroy removed everything: the lab's record goes too.
    Destroyed,
    /// The server started and found the lab owned by an operation that no longer runs.
    OwnerLost,
}

/// What an allowed `LabEvent` does to the lab's record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabTransition {
    /// The lab moves to this phase.
    To(LabPhase),
    /// The lab's record is removed.
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

    /// The expected result of every event from every phase.
    fn expected(phase: LabPhase, event: LabEvent) -> Result<LabTransition, LabRefused> {
        match (phase, event) {
            (Creating, LabEvent::Built) => Ok(To(Ready)),
            (Creating, LabEvent::Released) | (Destroying, LabEvent::Destroyed) => Ok(Removed),
            // A lab can always be destroyed, whatever state it was left in.
            (_, LabEvent::DestroyStarted) => Ok(To(Destroying)),
            (Destroying, LabEvent::DestroyFailed | LabEvent::OwnerLost) => Ok(To(DestroyFailed)),
            (Creating, LabEvent::OwnerLost) => Ok(To(Interrupted)),
            _ => refused(phase, event),
        }
    }

    #[test]
    fn every_event_from_every_phase() {
        for phase in LabPhase::iter() {
            for event in LabEvent::iter() {
                assert_eq!(
                    phase.on_event(event),
                    expected(phase, event),
                    "{event:?} from {phase:?}"
                );
            }
        }
    }

    #[test]
    fn only_a_ready_lab_takes_operations_on_its_nodes() {
        for phase in LabPhase::iter() {
            assert_eq!(phase.ensure_ready().is_ok(), phase == Ready, "{phase:?}");
            assert_eq!(phase.is_ready(), phase == Ready, "{phase:?}");
        }
    }

    // One test per defect in the state-machines inventory that the machine itself rules out.

    #[test]
    fn a_lab_being_created_cannot_be_built_or_released_twice() {
        // M3.1: only the `up` holding the reservation moves the lab out of `Creating`; once
        // built, a failed second `up` has no reservation to give back.
        assert_eq!(
            Ready.on_event(LabEvent::Released),
            refused(Ready, LabEvent::Released)
        );
        assert_eq!(
            Ready.on_event(LabEvent::Built),
            refused(Ready, LabEvent::Built)
        );
    }

    #[test]
    fn a_lab_left_mid_operation_by_a_restart_is_marked_not_left() {
        // M6.3: the daemon stopped while `up` built the lab, or while a destroy removed it.
        assert_eq!(Creating.on_event(LabEvent::OwnerLost), Ok(To(Interrupted)));
        assert_eq!(
            Destroying.on_event(LabEvent::OwnerLost),
            Ok(To(DestroyFailed))
        );
        for phase in [Interrupted, DestroyFailed] {
            assert_eq!(phase.on_event(LabEvent::DestroyStarted), Ok(To(Destroying)));
            let refusal = phase.ensure_ready().unwrap_err().to_string();
            assert!(refusal.contains("destroy it"), "{refusal}");
        }
    }

    #[test]
    fn a_half_destroyed_lab_is_not_ready() {
        // M6.2 and M7.3: a destroy or rollback that left resources behind used to leave a lab
        // that looked stopped and offered Start.
        assert_eq!(
            Destroying.on_event(LabEvent::DestroyFailed),
            Ok(To(DestroyFailed))
        );
        assert!(DestroyFailed.ensure_ready().is_err());
        assert_eq!(
            Ready.on_event(LabEvent::DestroyFailed),
            refused(Ready, LabEvent::DestroyFailed)
        );
    }

    #[test]
    fn a_phase_this_build_does_not_know_decodes_as_unknown() {
        let phase: LabPhase = serde_json::from_str("\"archived\"").unwrap();
        assert_eq!(phase, Unknown);
        assert!(phase.ensure_ready().is_err());
        assert_eq!(phase.on_event(LabEvent::DestroyStarted), Ok(To(Destroying)));
    }

    #[test]
    fn phases_are_stored_in_snake_case() {
        assert_eq!(
            serde_json::to_string(&DestroyFailed).unwrap(),
            "\"destroy_failed\""
        );
        for phase in LabPhase::iter().filter(|phase| *phase != Unknown) {
            let text = serde_json::to_string(&phase).unwrap();
            assert_eq!(serde_json::from_str::<LabPhase>(&text).unwrap(), phase);
        }
    }
}
