//! The P2p link ends of running nodes that the scanner could not read, kept in memory so each
//! problem is logged once.
//!
//! The kernel holds an end's real state, so nothing here is stored: the scanner records what it
//! finds on each scan of a running node's end. After sherpad starts, a lab has no entries until
//! its first scan.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use dashmap::DashMap;

use crate::services::error::Unexpected;

/// What could not be read of a link end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndRead {
    /// Its interface could not be looked up.
    LookUp,
    /// Its redirect filter could not be read.
    Filter,
}

/// A link end that could not be read: what failed, and the reference its log line has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadableEnd {
    pub read: EndRead,
    pub reference: Unexpected,
}

/// Each lab's unreadable link ends, by lab ID, then by host interface.
#[derive(Clone, Default)]
pub struct UnreadableEnds {
    labs: Arc<DashMap<String, HashMap<String, UnreadableEnd>>>,
}

impl fmt::Display for EndRead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EndRead::LookUp => write!(f, "look up"),
            EndRead::Filter => write!(f, "read the filter of"),
        }
    }
}

impl UnreadableEnds {
    /// Record end `iface` of lab `lab_id` as `unreadable`, or as read with `None`, and return
    /// what was recorded for it before.
    #[must_use]
    pub fn record(
        &self,
        lab_id: &str,
        iface: &str,
        unreadable: Option<UnreadableEnd>,
    ) -> Option<UnreadableEnd> {
        let mut lab = self.labs.entry(lab_id.to_string()).or_default();
        match unreadable {
            Some(unreadable) => lab.insert(iface.to_string(), unreadable),
            None => lab.remove(iface),
        }
    }

    /// What is recorded of end `iface` of lab `lab_id`.
    #[must_use]
    pub fn of_end(&self, lab_id: &str, iface: &str) -> Option<UnreadableEnd> {
        self.labs.get(lab_id)?.get(iface).cloned()
    }

    /// Forget lab `lab_id`, as it was destroyed.
    pub fn forget_lab(&self, lab_id: &str) {
        self.labs.remove(lab_id);
    }
}
