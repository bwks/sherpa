//! Long-running operations on a lab (`up`, `redeploy`, `resume`) and stopping them.
//!
//! An operation holds a read lease on its lab for as long as it runs, and checks the lab's
//! cancellation token at safe points: between phases and while it waits for nodes. Stopping a
//! lab's operations cancels the token, then takes the write lease, so it returns only after
//! every operation on that lab has ended. Operations stop only at those points, never mid-step,
//! because some steps hand work to blocking tasks that would outlive a dropped future.
//!
//! Operations are stopped for one of two reasons. Destroying a lab holds the write lease while
//! it removes the lab, so nothing writes to it afterwards; a stopped `up` leaves the removal to
//! the destroy. A stop requested by the user (Ctrl-C in the CLI) lets a stopped `up` roll back
//! like a failed one.
//!
//! The scanner observes a lab only while no operation holds it: `try_observe` takes the write
//! lease without waiting and without cancelling anything, so a lab with an operation running is
//! skipped for that cycle, and an operation that starts meanwhile waits until the scanner has
//! written what it saw.
//!
//! A lab's lock lives in `LAB_LEASES` only while something holds or waits for it. Each holder
//! and each waiter keeps a `LeaseEntry`, which counts it as a user of the entry; the last one to
//! drop removes the entry. The count is changed only through the map's entry API, so the check
//! and the removal happen under the shard lock that `lease` takes to join an entry: a caller is
//! either counted before the removal, or arrives after it and creates a fresh lock that nobody
//! else holds or waits for.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Result, bail};
use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::instrument;

static LAB_LEASES: OnceLock<DashMap<String, LeaseSlot>> = OnceLock::new();
static LAB_CANCELLATIONS: OnceLock<DashMap<String, Cancellation>> = OnceLock::new();
/// Destroys waiting for or holding each lab's write lease. An operation that starts while one
/// is pending, such as one queued behind a requested stop, is cancelled at once.
static PENDING_DESTROYS: OnceLock<DashMap<String, usize>> = OnceLock::new();

/// Why a lab's operations are being stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// The lab is being destroyed or cleaned.
    Destroy,
    /// The user asked to stop the operation.
    Request,
}

/// A lab's cancellation token and the reason it was cancelled.
#[derive(Clone, Default)]
struct Cancellation {
    token: CancellationToken,
    reason: Arc<OnceLock<StopReason>>,
}

/// A lab's lock in `LAB_LEASES`, and how many `LeaseEntry`s hold or wait on it.
struct LeaseSlot {
    lock: Arc<RwLock<()>>,
    users: usize,
}

/// One user's share of a lab's entry in `LAB_LEASES`. Dropping the last one removes the entry.
struct LeaseEntry {
    lab_id: String,
    lock: Arc<RwLock<()>>,
}

/// A running operation on a lab. Dropping it ends the operation's lease.
pub struct LabOperation {
    cancel: Cancellation,
    // Fields drop in declaration order: the guard must be declared before the entry, so the lock
    // is released before the entry can be removed and a later caller gets the same lock.
    _lease: OwnedRwLockReadGuard<()>,
    entry: LeaseEntry,
}

/// Exclusive access to a lab after its operations have stopped. Held while the lab is destroyed.
pub struct StoppedLab {
    reason: StopReason,
    // The guard must be declared before the entry: see `LabOperation`.
    _lease: OwnedRwLockWriteGuard<()>,
    entry: LeaseEntry,
}

/// The scanner's hold on a lab no operation holds. Node states are only observed under it.
pub struct ObservedLab {
    // The guard must be declared before the entry: see `LabOperation`.
    _lease: OwnedRwLockWriteGuard<()>,
    _entry: LeaseEntry,
}

/// Proof that the caller holds a lab's operation lease: an operation's shared lease, or the
/// exclusive one a destroy holds. A lab's phase is only written under one, and only for the lab
/// it is on. Sealed: only the lease types of this module implement it.
pub trait LabHold: sealed::Sealed {
    /// The lab the lease is on.
    fn lab_id(&self) -> &str;
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::LabOperation {}
    impl Sealed for super::StoppedLab {}
}

impl LabHold for LabOperation {
    fn lab_id(&self) -> &str {
        &self.entry.lab_id
    }
}

impl LabHold for StoppedLab {
    fn lab_id(&self) -> &str {
        &self.entry.lab_id
    }
}

impl Cancellation {
    fn cancel(&self, reason: StopReason) {
        // The first reason wins: a destroy that follows a requested stop finds it set.
        let _ = self.reason.set(reason);
        self.token.cancel();
    }
}

impl LabOperation {
    /// Why this operation must stop, or `None` while it may carry on.
    pub fn stop_reason(&self) -> Option<StopReason> {
        if self.cancel.token.is_cancelled() {
            self.cancel.reason.get().copied()
        } else {
            None
        }
    }

    /// Fail if this operation must stop. Call between steps.
    pub fn ensure_active(&self) -> Result<()> {
        match self.stop_reason() {
            None => Ok(()),
            Some(StopReason::Destroy) => bail!(
                "Lab {} is being destroyed; this operation stopped",
                self.entry.lab_id
            ),
            Some(StopReason::Request) => {
                bail!(
                    "Lab {}: this operation was stopped on request",
                    self.entry.lab_id
                )
            }
        }
    }

    /// Sleep between polls, ending early with an error if this operation must stop.
    pub async fn sleep(&self, duration: Duration) -> Result<()> {
        tokio::select! {
            () = tokio::time::sleep(duration) => Ok(()),
            () = self.cancel.token.cancelled() => self.ensure_active(),
        }
    }
}

impl LeaseEntry {
    /// Join a lab's entry in `LAB_LEASES`, creating it if no one holds or waits on the lab.
    fn join(lab_id: &str) -> Self {
        let mut slot = leases()
            .entry(lab_id.to_owned())
            .or_insert_with(|| LeaseSlot {
                lock: Arc::new(RwLock::new(())),
                users: 0,
            });
        slot.users += 1;
        Self {
            lab_id: lab_id.to_owned(),
            lock: Arc::clone(&slot.lock),
        }
    }
}

impl Drop for LeaseEntry {
    fn drop(&mut self) {
        // The count is read and the entry removed under one shard lock, and no other reference
        // into the map is held here, so this cannot deadlock against `join`.
        if let Entry::Occupied(mut slot) = leases().entry(self.lab_id.clone()) {
            slot.get_mut().users -= 1;
            if slot.get().users == 0 {
                slot.remove();
            }
        }
    }
}

impl Drop for StoppedLab {
    fn drop(&mut self) {
        if self.reason == StopReason::Destroy {
            pending_destroys().remove_if_mut(&self.entry.lab_id, |_, count| {
                *count -= 1;
                *count == 0
            });
        }
        // Operations started after the lab is gone get a fresh token.
        cancellations().remove(&self.entry.lab_id);
    }
}

fn leases() -> &'static DashMap<String, LeaseSlot> {
    LAB_LEASES.get_or_init(DashMap::new)
}

fn cancellations() -> &'static DashMap<String, Cancellation> {
    LAB_CANCELLATIONS.get_or_init(DashMap::new)
}

fn pending_destroys() -> &'static DashMap<String, usize> {
    PENDING_DESTROYS.get_or_init(DashMap::new)
}

/// Start an operation on a lab. Waits while the lab is being destroyed.
#[instrument(level = "debug")]
pub async fn begin(lab_id: &str) -> LabOperation {
    // The entry is taken before waiting, so a waiter cancelled mid-wait drops its count too.
    let entry = LeaseEntry::join(lab_id);
    let lease = Arc::clone(&entry.lock).read_owned().await;
    // Read the token after taking the lease: a destroy that has started has already cancelled it.
    let cancel = cancellations()
        .entry(lab_id.to_owned())
        .or_default()
        .clone();
    // A requested stop that ended before a waiting destroy has already removed the token the
    // destroy cancelled, so this operation must yield to that destroy here.
    if pending_destroys()
        .get(lab_id)
        .is_some_and(|count| *count > 0)
    {
        cancel.cancel(StopReason::Destroy);
    }
    LabOperation {
        cancel,
        _lease: lease,
        entry,
    }
}

/// Hold a lab for the scanner, or `None` when an operation holds it now. Never waits.
#[instrument(level = "debug")]
pub fn try_observe(lab_id: &str) -> Option<ObservedLab> {
    let entry = LeaseEntry::join(lab_id);
    Arc::clone(&entry.lock)
        .try_write_owned()
        .ok()
        .map(|lease| ObservedLab {
            _lease: lease,
            _entry: entry,
        })
}

/// Stop every operation on a lab and wait until they have finished.
///
/// Returns whether any operation was running, and the exclusive lease. A destroy holds it while
/// the lab's resources are removed; a requested stop drops it straight away.
#[instrument(level = "debug")]
pub async fn stop(lab_id: &str, reason: StopReason) -> (bool, StoppedLab) {
    if reason == StopReason::Destroy {
        *pending_destroys().entry(lab_id.to_owned()).or_default() += 1;
    }
    cancellations()
        .entry(lab_id.to_owned())
        .or_default()
        .cancel(reason);
    let entry = LeaseEntry::join(lab_id);
    let (was_running, guard) = match Arc::clone(&entry.lock).try_write_owned() {
        Ok(guard) => (false, guard),
        Err(_) => (true, Arc::clone(&entry.lock).write_owned().await),
    };
    (
        was_running,
        StoppedLab {
            reason,
            _lease: guard,
            entry,
        },
    )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    const WAIT: Duration = Duration::from_secs(5);

    /// How many holders and waiters a lab's entry counts, or 0 when it has none.
    fn users(lab_id: &str) -> usize {
        leases().get(lab_id).map_or(0, |slot| slot.users)
    }

    #[tokio::test]
    async fn an_operation_runs_until_its_lab_is_destroyed() {
        let operation = begin("lab-ops-active").await;
        assert!(operation.ensure_active().is_ok());
        assert!(operation.sleep(Duration::from_millis(1)).await.is_ok());
    }

    #[tokio::test]
    async fn stop_cancels_running_operations_and_waits_for_them() {
        let operation = begin("lab-ops-stop").await;
        let stopping = tokio::spawn(stop("lab-ops-stop", StopReason::Destroy));
        // The sleep ends early once the destroy has cancelled the operation.
        let error = tokio::time::timeout(WAIT, operation.sleep(Duration::from_secs(3600)))
            .await
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("being destroyed"));
        assert_eq!(operation.stop_reason(), Some(StopReason::Destroy));
        assert!(!stopping.is_finished());
        drop(operation);
        let (was_running, _stopped) = tokio::time::timeout(WAIT, stopping).await.unwrap().unwrap();
        assert!(was_running);
    }

    #[tokio::test]
    async fn an_operation_queued_behind_a_requested_stop_yields_to_a_destroy() {
        let lab = "lab-ops-queued";
        let running = begin(lab).await;
        let requested = tokio::spawn(stop(lab, StopReason::Request));
        tokio::time::sleep(Duration::from_millis(50)).await;
        // Queued behind the requested stop, then a destroy queues behind both.
        let queued = tokio::spawn(begin(lab));
        tokio::time::sleep(Duration::from_millis(50)).await;
        let destroying = tokio::spawn(stop(lab, StopReason::Destroy));
        tokio::time::sleep(Duration::from_millis(50)).await;

        drop(running);
        let (_, stopped) = tokio::time::timeout(WAIT, requested)
            .await
            .unwrap()
            .unwrap();
        drop(stopped);
        let queued = tokio::time::timeout(WAIT, queued).await.unwrap().unwrap();

        assert_eq!(queued.stop_reason(), Some(StopReason::Destroy));
        drop(queued);
        let (_, destroyed) = tokio::time::timeout(WAIT, destroying)
            .await
            .unwrap()
            .unwrap();
        drop(destroyed);
        // Once the destroy is done, new operations run again.
        assert_eq!(begin(lab).await.stop_reason(), None);
    }

    #[tokio::test]
    async fn stop_without_operations_does_not_wait() {
        let (was_running, _stopped) =
            tokio::time::timeout(WAIT, stop("lab-ops-idle", StopReason::Destroy))
                .await
                .unwrap();
        assert!(!was_running);
    }

    #[tokio::test]
    async fn operations_wait_for_a_destroy_and_then_start_fresh() {
        let (_, stopped) = stop("lab-ops-after", StopReason::Destroy).await;
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
        let (_, _stopped) = tokio::time::timeout(WAIT, stop("lab-ops-mine", StopReason::Destroy))
            .await
            .unwrap();
        assert!(other.ensure_active().is_ok());
    }

    #[tokio::test]
    async fn a_requested_stop_says_so_and_releases_the_lab() {
        let operation = begin("lab-ops-request").await;
        assert_eq!(operation.stop_reason(), None);
        let stopping = tokio::spawn(stop("lab-ops-request", StopReason::Request));
        let error = tokio::time::timeout(WAIT, operation.sleep(Duration::from_secs(3600)))
            .await
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("stopped on request"));
        assert_eq!(operation.stop_reason(), Some(StopReason::Request));
        drop(operation);
        let (was_running, stopped) = tokio::time::timeout(WAIT, stopping).await.unwrap().unwrap();
        assert!(was_running);
        drop(stopped);
        // Operations started after a requested stop run normally.
        assert!(begin("lab-ops-request").await.ensure_active().is_ok());
    }

    #[tokio::test]
    async fn a_destroy_after_a_requested_stop_keeps_the_first_reason() {
        let operation = begin("lab-ops-first").await;
        let request = tokio::spawn(stop("lab-ops-first", StopReason::Request));
        tokio::time::timeout(WAIT, operation.sleep(Duration::from_secs(3600)))
            .await
            .unwrap()
            .unwrap_err();
        let destroy = tokio::spawn(stop("lab-ops-first", StopReason::Destroy));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(operation.stop_reason(), Some(StopReason::Request));
        drop(operation);
        let _ = tokio::time::timeout(WAIT, request).await.unwrap().unwrap();
        let _ = tokio::time::timeout(WAIT, destroy).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn the_scanner_leaves_a_lab_alone_while_an_operation_runs() {
        // M1.1, M1.2 and M1.6: the scanner used to overwrite the states an operation wrote.
        let operation = begin("lab-ops-observe-busy").await;
        assert!(try_observe("lab-ops-observe-busy").is_none());
        drop(operation);
        assert!(try_observe("lab-ops-observe-busy").is_some());
    }

    #[tokio::test]
    async fn an_operation_waits_while_the_scanner_observes_its_lab() {
        let observed = try_observe("lab-ops-observe-wait").unwrap();
        let operation = tokio::spawn(async {
            let operation = begin("lab-ops-observe-wait").await;
            operation.ensure_active().is_ok()
        });
        tokio::task::yield_now().await;
        assert!(!operation.is_finished());
        drop(observed);
        assert!(
            tokio::time::timeout(WAIT, operation)
                .await
                .unwrap()
                .unwrap()
        );
    }

    #[tokio::test]
    async fn released_leases_leave_the_map_and_a_held_one_stays() {
        let held = begin("lab-ops-shrink-held").await;
        let labs: Vec<String> = (0..100).map(|i| format!("lab-ops-shrink-{i}")).collect();
        for lab in &labs {
            drop(begin(lab).await);
            drop(try_observe(lab));
            drop(stop(lab, StopReason::Request).await);
        }
        for lab in &labs {
            assert!(!leases().contains_key(lab), "{lab} was not removed");
        }
        assert!(leases().contains_key("lab-ops-shrink-held"));
        assert!(held.ensure_active().is_ok());
        drop(held);
        assert!(!leases().contains_key("lab-ops-shrink-held"));
    }

    #[tokio::test]
    async fn a_waiter_gets_the_lock_its_holder_released() {
        let lab = "lab-ops-same-lock";
        let (_, stopped) = stop(lab, StopReason::Request).await;
        let lock = Arc::clone(&stopped.entry.lock);
        let waiting = tokio::spawn(begin(lab));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiting.is_finished());
        assert_eq!(users(lab), 2);

        drop(stopped);
        let operation = tokio::time::timeout(WAIT, waiting).await.unwrap().unwrap();
        assert!(Arc::ptr_eq(&operation.entry.lock, &lock));
        assert_eq!(users(lab), 1);
        drop(operation);
        assert_eq!(users(lab), 0);
        assert!(!leases().contains_key(lab));
    }

    #[tokio::test]
    async fn a_waiter_cancelled_mid_wait_gives_up_its_count() {
        let lab = "lab-ops-cancelled-wait";
        let (_, stopped) = stop(lab, StopReason::Request).await;
        assert_eq!(users(lab), 1);

        // Both futures are dropped while they wait for the lease.
        assert!(
            tokio::time::timeout(Duration::from_millis(20), begin(lab))
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(20), stop(lab, StopReason::Request))
                .await
                .is_err()
        );
        assert_eq!(users(lab), 1);

        drop(stopped);
        assert_eq!(users(lab), 0);
        assert!(!leases().contains_key(lab));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn racing_takes_and_releases_never_split_a_lab_across_two_locks() {
        const LAB: &str = "lab-ops-race";
        static READERS: AtomicUsize = AtomicUsize::new(0);
        static WRITERS: AtomicUsize = AtomicUsize::new(0);

        async fn exclusive(round: usize) {
            assert_eq!(READERS.load(Ordering::SeqCst), 0, "round {round}");
            assert_eq!(WRITERS.fetch_add(1, Ordering::SeqCst), 0, "round {round}");
            tokio::task::yield_now().await;
            WRITERS.fetch_sub(1, Ordering::SeqCst);
        }

        let mut tasks = Vec::new();
        for _ in 0..8 {
            tasks.push(tokio::spawn(async {
                for _ in 0..200 {
                    let operation = begin(LAB).await;
                    assert_eq!(WRITERS.load(Ordering::SeqCst), 0);
                    READERS.fetch_add(1, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    READERS.fetch_sub(1, Ordering::SeqCst);
                    drop(operation);
                }
            }));
        }
        for writer in 0..4 {
            tasks.push(tokio::spawn(async move {
                for round in 0..100 {
                    // Half the writers wait for the lease, half only try for it.
                    if writer % 2 == 0 {
                        let (_, stopped) = stop(LAB, StopReason::Request).await;
                        exclusive(round).await;
                        drop(stopped);
                    } else if let Some(observed) = try_observe(LAB) {
                        exclusive(round).await;
                        drop(observed);
                    }
                }
            }));
        }
        for task in tasks {
            tokio::time::timeout(WAIT * 6, task).await.unwrap().unwrap();
        }
        assert_eq!(users(LAB), 0);
        assert!(!leases().contains_key(LAB));
    }
}
