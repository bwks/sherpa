//! The Vault nodes Sherpa manages that are sealed and that Sherpa cannot unseal, kept in memory
//! for `inspect` and the web UI.
//!
//! Vault holds a node's real seal state, so nothing here is stored: the scanner records what it
//! finds on each scan of a running Vault node, and a resume or redeploy what it left the node
//! in. After sherpad starts, a lab has no entries until its first scan.

use std::collections::HashMap;
use std::sync::Arc;

use dashmap::DashMap;

use shared::data::VaultSealed;

/// Each lab's sealed Vault nodes, by lab ID, then by node name.
#[derive(Clone, Default)]
pub struct VaultSeals {
    labs: Arc<DashMap<String, HashMap<String, VaultSealed>>>,
}

impl VaultSeals {
    /// Record node `node_name` of lab `lab_id` as `sealed`, or as not sealed with `None`, and
    /// return what was recorded for it before.
    #[must_use]
    pub fn record(
        &self,
        lab_id: &str,
        node_name: &str,
        sealed: Option<VaultSealed>,
    ) -> Option<VaultSealed> {
        let mut lab = self.labs.entry(lab_id.to_string()).or_default();
        match sealed {
            Some(sealed) => lab.insert(node_name.to_string(), sealed),
            None => lab.remove(node_name),
        }
    }

    /// The sealed Vault nodes of lab `lab_id`, by node name.
    #[must_use]
    pub fn of_lab(&self, lab_id: &str) -> HashMap<String, VaultSealed> {
        self.labs
            .get(lab_id)
            .map_or_else(HashMap::new, |nodes| nodes.value().clone())
    }

    /// Forget node `node_name` of lab `lab_id`, as it stopped.
    pub fn forget_node(&self, lab_id: &str, node_name: &str) {
        if let Some(mut lab) = self.labs.get_mut(lab_id) {
            lab.remove(node_name);
        }
    }

    /// Forget lab `lab_id`, as it was destroyed.
    pub fn forget_lab(&self, lab_id: &str) {
        self.labs.remove(lab_id);
    }
}
