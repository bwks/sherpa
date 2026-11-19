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
