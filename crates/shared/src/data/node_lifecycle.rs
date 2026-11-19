//! The node lifecycle: every allowed move between `NodeState`s, in one place.
//!
//! Two kinds of writer move a node, each with its own event type:
//!
//! - operations (`up`, `down`, `resume`, redeploy) with an `OperationEvent`, while they hold
//!   the lab's operation lease. An event the node's state does not allow is `Refused`.
//! - the scanner with an `Observation` of what libvirt or Docker reports, only while no
//!   operation holds the lab. An observation never fails: one the state does not take leaves
//!   it as it is. `Building` and `Failed` belong to operations, so observations never move
//!   them.
//!
//! The server writes the result with a compare-and-set on the state it read, so a writer that
//! raced another does not overwrite it.

use std::fmt;

use super::NodeState;

/// What libvirt or Docker reports for a node's domain or container. The server maps the
/// runtimes' own states to this in one table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runtime {
    /// The domain or container is running.
    Running,
    /// The domain or container exists and is not running.
    Stopped(Halt),
    /// The runtime reports a fault, in its own word: a crashed domain, a dead container.
    Faulted(&'static str),
    /// The runtime is between states, in its own word: shutting down, restarting, removing.
    Transitional(&'static str),
    /// There is no domain or container.
    Absent,
}

/// How a domain or container that is not running was left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Halt {
    /// Shut off, exited or never started: starting it boots it.
    Off,
    /// Paused: resuming it carries on where it was.
    Paused,
}

/// What the scanner's readiness probe found for a running node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    /// The node has no readiness probe the scanner can run: running counts as ready.
    NotProbed,
    /// The probe answered.
    Ready,
    /// The probe did not answer, or the node was not probed this cycle.
    NotReady,
}

/// What the scanner saw of a node in one cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observation {
    pub runtime: Runtime,
    pub probe: Probe,
}

/// A step an operation took with a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationEvent {
    /// A redeploy is about to tear the node down and build it again.
    RebuildStarted,
    /// The node's domain or container was started or resumed.
    Launched,
    /// The node's readiness check passed, or it has none.
    Ready,
    /// The node's domain or container was stopped.
    Stopped,
    /// The operation failed for the node.
    Failed,
}

/// An operation event the node's state does not allow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused {
    pub state: NodeState,
    pub event: OperationEvent,
}

impl fmt::Display for OperationEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OperationEvent::RebuildStarted => write!(f, "rebuild"),
            OperationEvent::Launched => write!(f, "start"),
            OperationEvent::Ready => write!(f, "mark ready"),
            OperationEvent::Stopped => write!(f, "stop"),
            OperationEvent::Failed => write!(f, "mark failed"),
        }
    }
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.state {
            NodeState::Building => write!(
                f,
                "the node is being built by another operation; wait for it to finish"
            ),
            NodeState::Missing => write!(f, "the node has no domain or container; redeploy it"),
            state => write!(f, "cannot {} a node that is {state}", self.event),
        }
    }
}

impl std::error::Error for Refused {}

impl NodeState {
    /// The state an operation event moves this one to, or why it may not.
    ///
    /// # Errors
    /// `Refused` when the event is not allowed from this state.
    pub fn on_operation(self, event: OperationEvent) -> Result<NodeState, Refused> {
        let refused = Err(Refused { state: self, event });
        match (self, event) {
            // Another operation is building the node: nothing else may touch it.
            (NodeState::Building, OperationEvent::RebuildStarted | OperationEvent::Stopped) => {
                refused
            }
            (_, OperationEvent::RebuildStarted) => Ok(NodeState::Building),
            // A node with no domain or container can only be redeployed.
            (NodeState::Missing, _) => refused,
            (_, OperationEvent::Launched) => Ok(NodeState::Starting),
            (NodeState::Starting | NodeState::Running, OperationEvent::Ready) => {
                Ok(NodeState::Running)
            }
            (_, OperationEvent::Ready) => refused,
            (_, OperationEvent::Stopped) => Ok(NodeState::Stopped),
            (_, OperationEvent::Failed) => Ok(NodeState::Failed),
        }
    }

    /// The state the scanner's observation moves this one to. Unchanged when the observation
    /// does not apply, as for a node an operation is building.
    #[must_use]
    pub fn on_observation(self, observation: Observation) -> NodeState {
        match (self, observation.runtime) {
            (NodeState::Building | NodeState::Failed, _) | (_, Runtime::Transitional(_)) => self,
            (_, Runtime::Absent) => NodeState::Missing,
            (_, Runtime::Faulted(_)) => NodeState::Failed,
            (_, Runtime::Stopped(_)) => NodeState::Stopped,
            (NodeState::Running, Runtime::Running) => NodeState::Running,
            (_, Runtime::Running) => match observation.probe {
                Probe::NotProbed | Probe::Ready => NodeState::Running,
                Probe::NotReady => NodeState::Starting,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use NodeState::{Building, Failed, Missing, Running, Starting, Stopped};

    fn refused(state: NodeState, event: OperationEvent) -> Result<NodeState, Refused> {
        Err(Refused { state, event })
    }

    const RUNTIMES: [Runtime; 7] = [
        Runtime::Running,
        Runtime::Stopped(Halt::Off),
        Runtime::Stopped(Halt::Paused),
        Runtime::Faulted("crashed"),
        Runtime::Transitional("restarting"),
        Runtime::Absent,
        Runtime::Faulted("dead"),
    ];

    #[test]
    fn operation_events_move_a_node_as_listed() {
        assert_eq!(Running.on_operation(OperationEvent::Stopped), Ok(Stopped));
        assert_eq!(Running.on_operation(OperationEvent::Failed), Ok(Failed));
        assert_eq!(Running.on_operation(OperationEvent::Ready), Ok(Running));
        assert_eq!(
            Building.on_operation(OperationEvent::Launched),
            Ok(Starting)
        );
        assert_eq!(
            Building.on_operation(OperationEvent::RebuildStarted),
            refused(Building, OperationEvent::RebuildStarted)
        );
        assert_eq!(
            Missing.on_operation(OperationEvent::RebuildStarted),
            Ok(Building)
        );
        assert_eq!(
            Missing.on_operation(OperationEvent::Stopped),
            refused(Missing, OperationEvent::Stopped)
        );
        assert_eq!(
            refused(Stopped, OperationEvent::Ready)
                .unwrap_err()
                .to_string(),
            "cannot mark ready a node that is stopped"
        );
    }

    #[test]
    fn observations_move_a_node_as_listed() {
        let seen = |runtime, probe| Observation { runtime, probe };
        assert_eq!(
            Running.on_observation(seen(Runtime::Transitional("restarting"), Probe::Ready)),
            Running
        );
        assert_eq!(
            Running.on_observation(seen(Runtime::Stopped(Halt::Paused), Probe::NotProbed)),
            Stopped
        );
        assert_eq!(
            Running.on_observation(seen(Runtime::Running, Probe::NotReady)),
            Running
        );
        assert_eq!(
            Stopped.on_observation(seen(Runtime::Running, Probe::NotProbed)),
            Running
        );
        assert_eq!(
            Missing.on_observation(seen(Runtime::Stopped(Halt::Off), Probe::NotProbed)),
            Stopped
        );
    }

    // One test per defect in the state-machines inventory that the machine itself rules out.

    #[test]
    fn a_node_being_built_is_not_seen_as_stopped() {
        // M1.1: `up` has inserted the node and not yet created its domain or container.
        for runtime in RUNTIMES {
            let observation = Observation {
                runtime,
                probe: Probe::NotProbed,
            };
            assert_eq!(Building.on_observation(observation), Building);
        }
    }

    #[test]
    fn a_failed_node_stays_failed_until_an_operation_moves_it() {
        // M1.2: the scanner used to overwrite `Failed` on its next cycle.
        for runtime in RUNTIMES {
            let observation = Observation {
                runtime,
                probe: Probe::Ready,
            };
            assert_eq!(Failed.on_observation(observation), Failed);
        }
        assert_eq!(Failed.on_operation(OperationEvent::Launched), Ok(Starting));
        assert_eq!(
            Failed.on_operation(OperationEvent::RebuildStarted),
            Ok(Building)
        );
    }

    #[test]
    fn running_means_ready() {
        // M1.3: a started node is `Starting`; only a readiness check makes it `Running`.
        assert_eq!(Stopped.on_operation(OperationEvent::Launched), Ok(Starting));
        let unprobed_start = Observation {
            runtime: Runtime::Running,
            probe: Probe::NotReady,
        };
        assert_eq!(Stopped.on_observation(unprobed_start), Starting);
        assert_eq!(Starting.on_observation(unprobed_start), Starting);
        assert_eq!(Starting.on_operation(OperationEvent::Ready), Ok(Running));
    }

    #[test]
    fn a_crashed_or_dead_node_is_failed_not_stopped() {
        // M1.4 and M4.1: a crashed domain or a dead container cannot be started.
        for word in ["crashed", "dead"] {
            let observation = Observation {
                runtime: Runtime::Faulted(word),
                probe: Probe::NotProbed,
            };
            assert_eq!(Running.on_observation(observation), Failed);
            assert_eq!(Stopped.on_observation(observation), Failed);
        }
    }

    #[test]
    fn down_cannot_stop_a_node_an_operation_is_building() {
        // M1.5: `down` during `up` used to be overtaken by `up` starting the node afterwards.
        let refusal = Building.on_operation(OperationEvent::Stopped);
        assert_eq!(refusal, refused(Building, OperationEvent::Stopped));
        assert!(
            refusal
                .err()
                .is_some_and(|refusal| refusal.to_string().contains("being built"))
        );
    }

    #[test]
    fn a_stopped_node_cannot_be_marked_ready() {
        // M1.5: `up` or `resume` probed the node, then `down` stopped it before the write.
        assert_eq!(
            Stopped.on_operation(OperationEvent::Ready),
            refused(Stopped, OperationEvent::Ready)
        );
    }

    #[test]
    fn a_node_with_no_runtime_object_is_missing_not_stopped() {
        // M4.2: a missing domain or container used to look stopped, so the UI offered Start.
        let gone = Observation {
            runtime: Runtime::Absent,
            probe: Probe::NotProbed,
        };
        assert_eq!(Running.on_observation(gone), Missing);
        let refusal = Missing.on_operation(OperationEvent::Launched);
        assert!(
            refusal
                .err()
                .is_some_and(|refusal| refusal.to_string().contains("redeploy it"))
        );
    }
}
