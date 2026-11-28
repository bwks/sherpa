//! One lock per Docker store, the disk a VM makes once for sandboxes, so the sandboxes that need
//! it wait for the one VM that makes it (`docs/architecture/SANDBOX.md`).

use std::sync::Arc;

use anyhow::{Context, Result};
use dashmap::DashMap;
use tokio::sync::Mutex;
use tokio::task;

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

    /// Run `build` of volume `volume` in a task that holds the volume's lock until the build
    /// ends, even when the caller stops waiting, calling `waiting` first when another holds it.
    ///
    /// # Errors
    ///
    /// Fails when the build fails or panics.
    pub async fn build<T, B>(&self, volume: &str, waiting: impl FnOnce(), build: B) -> Result<T>
    where
        T: Send + 'static,
        B: Future<Output = Result<T>> + Send + 'static,
    {
        let lock = self.of(volume);
        let guard = if let Ok(guard) = Arc::clone(&lock).try_lock_owned() {
            guard
        } else {
            waiting();
            lock.lock_owned().await
        };
        task::spawn(async move {
            let outcome = build.await;
            drop(guard);
            outcome
        })
        .await
        .with_context(|| format!("The build of {volume} panicked"))?
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::oneshot;
    use tokio::time;

    use super::*;

    #[tokio::test]
    async fn sandbox_089_a_disk_build_whose_caller_stops_waiting_holds_the_disk_lock_until_it_ends()
    {
        let locks = SandboxDiskLocks::default();
        let (started_tx, started_rx) = oneshot::channel();
        let (finish_tx, finish_rx) = oneshot::channel::<()>();
        let (ended_tx, ended_rx) = oneshot::channel();
        let build = async move {
            started_tx.send(()).unwrap();
            finish_rx.await.unwrap();
            ended_tx.send(()).unwrap();
            Ok(())
        };
        let caller = tokio::spawn({
            let locks = locks.clone();
            async move { locks.build("disk.qcow2", || {}, build).await }
        });
        started_rx.await.unwrap();
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());

        let lock = locks.of("disk.qcow2");
        assert!(lock.try_lock().is_err());
        finish_tx.send(()).unwrap();
        ended_rx.await.unwrap();
        let _released = time::timeout(Duration::from_secs(5), lock.lock())
            .await
            .unwrap();
    }
}
