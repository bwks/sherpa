//! Long-running operations on a lab (`up`, `redeploy`, `resume`) and stopping them.
//!
//! An operation holds a read lease on its lab for as long as it runs, and checks the lab's
//! cancellation token at safe points: between phases and while it waits for nodes. Destroying
//! a lab cancels the token, then takes the write lease, so it starts only after every operation
//! on that lab has stopped and none can write to the lab afterwards. Operations stop only at
//! those points, never mid-step, because some steps hand work to blocking tasks that would
//! outlive a dropped future.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Result, bail};
use dashmap::DashMap;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::instrument;

static LAB_LEASES: OnceLock<DashMap<String, Arc<RwLock<()>>>> = OnceLock::new();
static LAB_CANCELLATIONS: OnceLock<DashMap<String, CancellationToken>> = OnceLock::new();

/// A running operation on a lab. Dropping it ends the operation's lease.
pub struct LabOperation {
    lab_id: String,
    cancel: CancellationToken,
    _lease: OwnedRwLockReadGuard<()>,
}

/// Exclusive access to a lab after its operations have stopped. Held while the lab is destroyed.
pub struct StoppedLab {
    lab_id: String,
    _lease: OwnedRwLockWriteGuard<()>,
}

impl LabOperation {
    /// Whether the lab is being destroyed and this operation must stop.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Fail if the lab is being destroyed. Call between steps.
    pub fn ensure_active(&self) -> Result<()> {
        if self.is_cancelled() {
            bail!(
                "Lab {} is being destroyed; this operation stopped",
                self.lab_id
            );
        }
        Ok(())
    }

    /// Sleep between polls, ending early with an error if the lab is being destroyed.
    pub async fn sleep(&self, duration: Duration) -> Result<()> {
        tokio::select! {
            () = tokio::time::sleep(duration) => Ok(()),
            () = self.cancel.cancelled() => self.ensure_active(),
        }
    }
}

impl Drop for StoppedLab {
    fn drop(&mut self) {
        // Operations started after the lab is gone get a fresh token.
        cancellations().remove(&self.lab_id);
    }
}

fn leases() -> &'static DashMap<String, Arc<RwLock<()>>> {
    LAB_LEASES.get_or_init(DashMap::new)
}

fn cancellations() -> &'static DashMap<String, CancellationToken> {
    LAB_CANCELLATIONS.get_or_init(DashMap::new)
}

fn lease(lab_id: &str) -> Arc<RwLock<()>> {
    leases()
        .entry(lab_id.to_owned())
        .or_insert_with(|| Arc::new(RwLock::new(())))
        .clone()
}

/// Start an operation on a lab. Waits while the lab is being destroyed.
#[instrument(level = "debug")]
pub async fn begin(lab_id: &str) -> LabOperation {
    let lease = lease(lab_id).read_owned().await;
    // Read the token after taking the lease: a destroy that has started has already cancelled it.
    let cancel = cancellations()
        .entry(lab_id.to_owned())
        .or_default()
        .clone();
    LabOperation {
        lab_id: lab_id.to_owned(),
        cancel,
        _lease: lease,
    }
}

/// Stop every operation on a lab and wait until they have finished.
///
/// Returns whether any operation was running, and the exclusive lease to hold while the lab's
/// resources are removed.
#[instrument(level = "debug")]
pub async fn stop(lab_id: &str) -> (bool, StoppedLab) {
    cancellations()
        .entry(lab_id.to_owned())
        .or_default()
        .cancel();
    let lease = lease(lab_id);
    let (was_running, guard) = match lease.clone().try_write_owned() {
        Ok(guard) => (false, guard),
        Err(_) => (true, lease.write_owned().await),
    };
    (
        was_running,
        StoppedLab {
            lab_id: lab_id.to_owned(),
            _lease: guard,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAIT: Duration = Duration::from_secs(5);

    #[tokio::test]
    async fn an_operation_runs_until_its_lab_is_destroyed() {
        let operation = begin("lab-ops-active").await;
        assert!(operation.ensure_active().is_ok());
        assert!(operation.sleep(Duration::from_millis(1)).await.is_ok());
    }

    #[tokio::test]
    async fn stop_cancels_running_operations_and_waits_for_them() {
        let operation = begin("lab-ops-stop").await;
        let stopping = tokio::spawn(stop("lab-ops-stop"));
        // The sleep ends early once the destroy has cancelled the operation.
        let error = tokio::time::timeout(WAIT, operation.sleep(Duration::from_secs(3600)))
            .await
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("being destroyed"));
        assert!(operation.ensure_active().is_err());
        assert!(!stopping.is_finished());
        drop(operation);
        let (was_running, _stopped) = tokio::time::timeout(WAIT, stopping).await.unwrap().unwrap();
        assert!(was_running);
    }

    #[tokio::test]
    async fn stop_without_operations_does_not_wait() {
        let (was_running, _stopped) = tokio::time::timeout(WAIT, stop("lab-ops-idle"))
            .await
            .unwrap();
        assert!(!was_running);
    }

    #[tokio::test]
    async fn operations_wait_for_a_destroy_and_then_start_fresh() {
        let (_, stopped) = stop("lab-ops-after").await;
        let starting = tokio::spawn(async {
            let operation = begin("lab-ops-after").await;
            operation.ensure_active().is_ok()
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!starting.is_finished());
        drop(stopped);
        assert!(tokio::time::timeout(WAIT, starting).await.unwrap().unwrap());
    }

    #[tokio::test]
    async fn stop_only_affects_its_own_lab() {
        let other = begin("lab-ops-other").await;
        let (_, _stopped) = tokio::time::timeout(WAIT, stop("lab-ops-mine"))
            .await
            .unwrap();
        assert!(other.ensure_active().is_ok());
    }
}
