//! One lock per disk that a VM makes once for sandboxes, a boot disk or a Docker store, so the
//! sandboxes that need it wait for the one VM that makes it. See `docs/architecture/SANDBOX.md`.

use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::Mutex;

/// The lock of each disk a VM makes for sandboxes, by volume name.
#[derive(Clone, Default)]
pub struct SandboxDiskLocks {
    disks: Arc<DashMap<String, Arc<Mutex<()>>>>,
}

impl SandboxDiskLocks {
    /// The lock of volume `volume`, made on first use.
    #[must_use]
    pub fn of(&self, volume: &str) -> Arc<Mutex<()>> {
        Arc::clone(&self.disks.entry(volume.to_owned()).or_default())
    }
}
