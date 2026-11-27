//! One lock per sandbox Docker store, so the nodes of one image wait for the one VM that makes
//! its store. See `docs/architecture/SANDBOX.md`.

use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::Mutex;

/// The lock of each Docker store, by store name.
#[derive(Clone, Default)]
pub struct DockerStoreLocks {
    stores: Arc<DashMap<String, Arc<Mutex<()>>>>,
}

impl DockerStoreLocks {
    /// The lock of store `store`, made on first use.
    #[must_use]
    pub fn of(&self, store: &str) -> Arc<Mutex<()>> {
        Arc::clone(&self.stores.entry(store.to_owned()).or_default())
    }
}
