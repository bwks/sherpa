//! The last known state of each lab's P2p links, kept in memory for `inspect`.
//!
//! The kernel holds a link's real state and the database what the link should be, so nothing
//! here is stored: the scanner rebuilds a lab's states on each scan it makes of the lab, and a
//! resume or redeploy records the links it re-attached. After sherpad starts, a lab has no
//! states until its first scan, and `inspect` reports its links as not checked.

use std::collections::HashMap;
use std::sync::Arc;

use dashmap::DashMap;

use shared::data::LinkState;

/// Each lab's link states, by lab ID, then by link index.
#[derive(Clone, Default)]
pub struct LinkStates {
    labs: Arc<DashMap<String, HashMap<u16, LinkState>>>,
}

impl LinkStates {
    /// Replace every link state of lab `lab_id` with `states`, as a scan saw them.
    pub fn replace_lab(&self, lab_id: &str, states: HashMap<u16, LinkState>) {
        self.labs.insert(lab_id.to_string(), states);
    }

    /// Record the state of link `index` of lab `lab_id`.
    pub fn record(&self, lab_id: &str, index: u16, state: LinkState) {
        self.labs
            .entry(lab_id.to_string())
            .or_default()
            .insert(index, state);
    }

    /// The link states of lab `lab_id`; empty before it is first scanned.
    #[must_use]
    pub fn of_lab(&self, lab_id: &str) -> HashMap<u16, LinkState> {
        self.labs
            .get(lab_id)
            .map_or_else(HashMap::new, |states| states.value().clone())
    }

    /// Forget lab `lab_id`, as it was destroyed.
    pub fn forget_lab(&self, lab_id: &str) {
        self.labs.remove(lab_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lab_has_no_states_until_they_are_recorded() {
        let states = LinkStates::default();
        assert!(states.of_lab("lab1").is_empty());
        states.record("lab1", 2, LinkState::Attached);
        assert_eq!(
            states.of_lab("lab1"),
            HashMap::from([(2, LinkState::Attached)])
        );
        assert!(states.of_lab("lab2").is_empty());
    }

    #[test]
    fn a_scan_replaces_every_state_of_its_lab() {
        let states = LinkStates::default();
        states.record("lab1", 0, LinkState::Attached);
        states.record("lab1", 1, LinkState::Attached);
        let peer_down = LinkState::PeerDown {
            peer: "vm2".to_string(),
        };
        states.replace_lab("lab1", HashMap::from([(0, peer_down.clone())]));
        assert_eq!(states.of_lab("lab1"), HashMap::from([(0, peer_down)]));
        states.forget_lab("lab1");
        assert!(states.of_lab("lab1").is_empty());
    }
}
