//! Operations on a lab, what holds each lab's lease, and stopping them.
//!
//! These hold a lab's lease, each as a [`LeaseHolder`] recorded with the lab's entry:
//! - the operations of [`Operation`], each holding a shared read lease for as long as it runs:
//!   `up`, `redeploy`, `resume`, `down`, and the recovery of a lab at startup;
//! - a destroy or a clean, and a requested stop, each holding the write lease alone;
//! - the scanner's observation, holding the write lease alone without waiting for it.
//!
//! `up`, `redeploy` and `resume` check the lab's cancellation token at safe points: between
//! phases and while they wait for nodes. They stop only at those points, never mid-step, because
//! some steps hand work to blocking tasks that would outlive a dropped future. `down` checks the
//! token once, before it starts, so a destroy that is under way stops it; once running it is
//! never stopped. Recovery never checks it.
//!
//! Operations are stopped for one of two reasons. Destroying a lab cancels the token, then takes
//! the write lease and holds it while it removes the lab, so it returns only after every
//! operation on that lab has ended and nothing writes to it afterwards; a stopped `up` leaves
//! the removal to the destroy. A stop requested by the user (Ctrl-C in the CLI) is made only
//! when an `up`, `redeploy` or `resume` holds or waits for the lease: it cancels the token, waits
//! for the write lease and lets a stopped `up` roll back like a failed one. When only a `down`,
//! a recovery, a destroy or the scanner holds the lab, a requested stop returns at once, having
//! cancelled nothing.
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

use crate::services::error::ServiceError;

static LAB_LEASES: OnceLock<DashMap<String, LeaseSlot>> = OnceLock::new();
static LAB_CANCELLATIONS: OnceLock<DashMap<String, Cancellation>> = OnceLock::new();
/// Destroys waiting for or holding each lab's write lease. An operation that starts while one
/// is pending, such as one queued behind a requested stop, is cancelled at once.
static PENDING_DESTROYS: OnceLock<DashMap<String, usize>> = OnceLock::new();

/// An operation that runs under a lab's shared lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Up,
    Redeploy,
    Resume,
    Down,
    Recovery,
}

/// What holds or waits for a lab's lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaseHolder {
    Operation(Operation),
    Stop(StopReason),
    Scanner,
}

/// Why a lab's operations are being stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    Destroy,
    Request,
}

/// A lab's cancellation token and the reason it was cancelled.
#[derive(Clone, Default)]
struct Cancellation {
    token: CancellationToken,
    reason: Arc<OnceLock<StopReason>>,
}

/// A lab's lock in `LAB_LEASES`, and what each `LeaseEntry` holding or waiting on it is.
struct LeaseSlot {
    lock: Arc<RwLock<()>>,
    holders: Vec<LeaseHolder>,
}

/// One holder's share of a lab's entry in `LAB_LEASES`. Dropping the last one removes the entry.
struct LeaseEntry {
    lab_id: String,
    holder: LeaseHolder,
    lock: Arc<RwLock<()>>,
}

/// A running operation on a lab. Dropping it ends the operation's lease. Fields drop in
/// declaration order, so `_lease` must stay before `entry`: the lock is then released before the
/// entry can be removed, and a later caller cannot make a fresh lock while this guard is held.
pub struct LabOperation {
    cancel: Cancellation,
    _lease: OwnedRwLockReadGuard<()>,
    entry: LeaseEntry,
}

/// Exclusive access to a lab after its operations have stopped. Held while the lab is destroyed.
/// `_lease` must stay before `entry` so the lock is released before the entry can be removed, as
/// in `LabOperation`.
pub struct StoppedLab {
    reason: StopReason,
    _lease: OwnedRwLockWriteGuard<()>,
    entry: LeaseEntry,
}

/// The scanner's hold on a lab no operation holds. Node states are only observed under it.
/// `_lease` must stay before `_entry` so the lock is released before the entry can be removed, as
/// in `LabOperation`.
pub struct ObservedLab {
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

impl Operation {
    /// Whether a requested stop ends this operation: only these check the lab's cancellation
    /// token while they run.
    fn stops_on_request(self) -> bool {
        matches!(
            self,
            Operation::Up | Operation::Redeploy | Operation::Resume
        )
    }
}

impl LeaseHolder {
    /// Whether this holder is an operation a requested stop ends.
    fn stops_on_request(self) -> bool {
        matches!(self, LeaseHolder::Operation(operation) if operation.stops_on_request())
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
            Some(StopReason::Destroy) => bail!(ServiceError::LabBeingDestroyed {
                lab_id: self.entry.lab_id.clone()
            }),
            Some(StopReason::Request) => bail!(ServiceError::OperationStoppedOnRequest {
                lab_id: self.entry.lab_id.clone()
            }),
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
    /// Join a lab's entry in `LAB_LEASES` as `holder`, creating it if no one holds or waits on
    /// the lab.
    fn join(lab_id: &str, holder: LeaseHolder) -> Self {
        let mut slot = leases()
            .entry(lab_id.to_owned())
            .or_insert_with(|| LeaseSlot {
                lock: Arc::new(RwLock::new(())),
                holders: Vec::new(),
            });
        slot.holders.push(holder);
        Self {
            lab_id: lab_id.to_owned(),
            holder,
            lock: Arc::clone(&slot.lock),
        }
    }

    /// Join a lab's entry as a requested stop, or `None` when no operation a requested stop ends
    /// holds or waits for the lab. The check and the join happen under one shard lock.
    fn join_stop_request(lab_id: &str) -> Option<Self> {
        let mut slot = leases().get_mut(lab_id)?;
        if !slot
            .holders
            .iter()
            .copied()
            .any(LeaseHolder::stops_on_request)
        {
            return None;
        }
        let holder = LeaseHolder::Stop(StopReason::Request);
        slot.holders.push(holder);
        Some(Self {
            lab_id: lab_id.to_owned(),
            holder,
            lock: Arc::clone(&slot.lock),
        })
    }
}

impl Drop for LeaseEntry {
    fn drop(&mut self) {
        // The count is read and the entry removed under one shard lock, and no other reference
        // into the map is held here, so this cannot deadlock against `join`.
        if let Entry::Occupied(mut slot) = leases().entry(self.lab_id.clone()) {
            let holders = &mut slot.get_mut().holders;
            if let Some(index) = holders.iter().position(|holder| *holder == self.holder) {
                holders.swap_remove(index);
            }
            if holders.is_empty() {
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

/// Start `operation` on a lab. Waits while the lab is being destroyed.
///
/// The lease it takes is a shared read lease, so operations on one lab can run at once, two
/// `up`s or an `up` and a `resume` among them. What keeps them apart is the lab's phase, not this
/// lease: the second `up` cannot reserve the lab, and `resume` refuses a lab that is not `Ready`.
/// `REDEPLOY_LOCKS` serializes a lab's redeploys with each other only.
#[instrument(level = "debug")]
pub async fn begin(lab_id: &str, operation: Operation) -> LabOperation {
    // The entry is taken before waiting, so a waiter cancelled mid-wait drops its share too, and
    // a requested stop finds the operation while it waits.
    let entry = LeaseEntry::join(lab_id, LeaseHolder::Operation(operation));
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
    let entry = LeaseEntry::join(lab_id, LeaseHolder::Scanner);
    Arc::clone(&entry.lock)
        .try_write_owned()
        .ok()
        .map(|lease| ObservedLab {
            _lease: lease,
            _entry: entry,
        })
}

/// Stop every operation on a lab for a destroy, and wait until they have finished.
///
/// Returns whether anything held the lab, and the exclusive lease, which the destroy holds
/// while the lab's resources are removed.
#[instrument(level = "debug")]
pub async fn stop(lab_id: &str) -> (bool, StoppedLab) {
    *pending_destroys().entry(lab_id.to_owned()).or_default() += 1;
    cancellations()
        .entry(lab_id.to_owned())
        .or_default()
        .cancel(StopReason::Destroy);
    let entry = LeaseEntry::join(lab_id, LeaseHolder::Stop(StopReason::Destroy));
    let (was_running, guard) = match Arc::clone(&entry.lock).try_write_owned() {
        Ok(guard) => (false, guard),
        Err(_) => (true, Arc::clone(&entry.lock).write_owned().await),
    };
    (
        was_running,
        StoppedLab {
            reason: StopReason::Destroy,
            _lease: guard,
            entry,
        },
    )
}

/// Stop the `up`, `redeploy` and `resume` running on a lab, on the user's request, and wait
/// until everything holding the lab has let it go.
///
/// Returns whether such an operation held or waited for the lab. When none does, it returns at
/// once and cancels nothing: a `down`, a recovery, a destroy or the scanner holding the lab
/// carries on.
#[instrument(level = "debug")]
pub async fn request_stop(lab_id: &str) -> bool {
    let Some(entry) = LeaseEntry::join_stop_request(lab_id) else {
        return false;
    };
    cancellations()
        .entry(lab_id.to_owned())
        .or_default()
        .cancel(StopReason::Request);
    let guard = Arc::clone(&entry.lock).write_owned().await;
    drop(StoppedLab {
        reason: StopReason::Request,
        _lease: guard,
        entry,
    });
    true
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    const WAIT: Duration = Duration::from_secs(5);

    #[tokio::test]
    async fn lab_concurrency_005_a_destroy_stops_the_operations_of_its_lab_and_waits_for_them() {
        let operation = begin("lab-ops-stop", Operation::Up).await;
        let stopping = tokio::spawn(stop("lab-ops-stop"));
        // The sleep ends early once the destroy has cancelled the operation.
        let error = tokio::time::timeout(WAIT, operation.sleep(Duration::from_secs(3600)))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<ServiceError>(),
            Some(&ServiceError::LabBeingDestroyed {
                lab_id: "lab-ops-stop".to_owned()
            })
        );
        assert_eq!(operation.stop_reason(), Some(StopReason::Destroy));
        assert!(!stopping.is_finished());
        drop(operation);
        let (was_running, _stopped) = tokio::time::timeout(WAIT, stopping).await.unwrap().unwrap();
        assert!(was_running);
    }

    #[tokio::test]
    async fn lab_concurrency_009_an_operation_queued_behind_a_requested_stop_yields_to_a_destroy() {
        let lab = "lab-ops-queued";
        let running = begin(lab, Operation::Up).await;
        let requested = tokio::spawn(request_stop(lab));
        tokio::time::sleep(Duration::from_millis(50)).await;
        // Queued behind the requested stop, then a destroy queues behind both.
        let queued = tokio::spawn(begin(lab, Operation::Resume));
        tokio::time::sleep(Duration::from_millis(50)).await;
        let destroying = tokio::spawn(stop(lab));
        tokio::time::sleep(Duration::from_millis(50)).await;

        drop(running);
        assert!(
            tokio::time::timeout(WAIT, requested)
                .await
                .unwrap()
                .unwrap()
        );
        let queued = tokio::time::timeout(WAIT, queued).await.unwrap().unwrap();

        assert_eq!(queued.stop_reason(), Some(StopReason::Destroy));
        drop(queued);
        let (_, destroyed) = tokio::time::timeout(WAIT, destroying)
            .await
            .unwrap()
            .unwrap();
        drop(destroyed);
        // Once the destroy is done, new operations run again.
        assert_eq!(begin(lab, Operation::Up).await.stop_reason(), None);
    }

    #[tokio::test]
    async fn lab_concurrency_006_a_destroy_with_nothing_running_does_not_wait() {
        let (was_running, _stopped) = tokio::time::timeout(WAIT, stop("lab-ops-idle"))
            .await
            .unwrap();
        assert!(!was_running);
    }

    #[tokio::test]
    async fn lab_concurrency_007_an_operation_waits_for_a_destroy_then_runs() {
        let (_, stopped) = stop("lab-ops-after").await;
        let starting = tokio::spawn(async {
            let operation = begin("lab-ops-after", Operation::Up).await;
            operation.ensure_active().is_ok()
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!starting.is_finished());
        drop(stopped);
        assert!(tokio::time::timeout(WAIT, starting).await.unwrap().unwrap());
    }

    #[tokio::test]
    async fn lab_concurrency_008_stopping_one_labs_operations_leaves_another_running() {
        let other = begin("lab-ops-other", Operation::Up).await;
        let (_, _stopped) = tokio::time::timeout(WAIT, stop("lab-ops-mine"))
            .await
            .unwrap();
        assert!(other.ensure_active().is_ok());
    }

    #[tokio::test]
    async fn lab_cancel_003_a_requested_stop_ends_the_operation_and_releases_the_lab() {
        let operation = begin("lab-ops-request", Operation::Up).await;
        assert_eq!(operation.stop_reason(), None);
        let stopping = tokio::spawn(request_stop("lab-ops-request"));
        let error = tokio::time::timeout(WAIT, operation.sleep(Duration::from_secs(3600)))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<ServiceError>(),
            Some(&ServiceError::OperationStoppedOnRequest {
                lab_id: "lab-ops-request".to_owned()
            })
        );
        assert_eq!(operation.stop_reason(), Some(StopReason::Request));
        drop(operation);
        assert!(tokio::time::timeout(WAIT, stopping).await.unwrap().unwrap());
        // Operations started after a requested stop run normally.
        assert!(
            begin("lab-ops-request", Operation::Up)
                .await
                .ensure_active()
                .is_ok()
        );
    }

    #[tokio::test]
    async fn lab_cancel_012_a_cancel_during_a_down_answers_stopped_false_at_once_and_the_down_runs_on()
     {
        let down = begin("lab-ops-down", Operation::Down).await;
        let stopped = tokio::time::timeout(WAIT, request_stop("lab-ops-down"))
            .await
            .unwrap();
        assert!(!stopped);
        assert_eq!(down.stop_reason(), None);
        assert!(down.ensure_active().is_ok());
    }

    #[tokio::test]
    async fn lab_cancel_013_a_cancel_during_a_scan_of_an_idle_lab_answers_stopped_false() {
        let observed = try_observe("lab-ops-scan").unwrap();
        let stopped = tokio::time::timeout(WAIT, request_stop("lab-ops-scan"))
            .await
            .unwrap();
        assert!(!stopped);
        drop(observed);
        // Nothing was cancelled: the next operation runs.
        assert!(
            begin("lab-ops-scan", Operation::Up)
                .await
                .ensure_active()
                .is_ok()
        );
    }

    #[tokio::test]
    async fn lab_cancel_014_a_cancel_of_an_up_waiting_behind_a_scan_stops_it_and_answers_stopped_true()
     {
        let lab = "lab-ops-up-behind-scan";
        let observed = try_observe(lab).unwrap();
        let up = tokio::spawn(async move { begin(lab, Operation::Up).await.ensure_active() });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let stopping = tokio::spawn(request_stop(lab));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!stopping.is_finished());
        drop(observed);
        let error = tokio::time::timeout(WAIT, up)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<ServiceError>(),
            Some(&ServiceError::OperationStoppedOnRequest {
                lab_id: lab.to_owned()
            })
        );
        assert!(tokio::time::timeout(WAIT, stopping).await.unwrap().unwrap());
    }

    #[tokio::test]
    async fn lab_concurrency_010_a_destroy_after_a_requested_stop_keeps_the_first_reason() {
        let operation = begin("lab-ops-first", Operation::Up).await;
        let request = tokio::spawn(request_stop("lab-ops-first"));
        tokio::time::timeout(WAIT, operation.sleep(Duration::from_secs(3600)))
            .await
            .unwrap()
            .unwrap_err();
        let destroy = tokio::spawn(stop("lab-ops-first"));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(operation.stop_reason(), Some(StopReason::Request));
        drop(operation);
        let _ = tokio::time::timeout(WAIT, request).await.unwrap().unwrap();
        let _ = tokio::time::timeout(WAIT, destroy).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn lab_concurrency_011_an_operation_waits_while_the_scanner_observes_its_lab() {
        let observed = try_observe("lab-ops-observe-wait").unwrap();
        let operation = tokio::spawn(async {
            let operation = begin("lab-ops-observe-wait", Operation::Up).await;
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn lab_concurrency_012_racing_operations_stops_and_scans_never_share_a_lab() {
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
                    let operation = begin(LAB, Operation::Up).await;
                    assert_eq!(WRITERS.load(Ordering::SeqCst), 0);
                    READERS.fetch_add(1, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    READERS.fetch_sub(1, Ordering::SeqCst);
                    drop(operation);
                }
            }));
        }
        for writer in 0..6 {
            tasks.push(tokio::spawn(async move {
                for round in 0..100 {
                    // A third of the writers wait for the lease, a third only try for it, and a
                    // third ask for a stop, which holds it alone too.
                    if writer % 3 == 0 {
                        let (_, stopped) = stop(LAB).await;
                        exclusive(round).await;
                        drop(stopped);
                    } else if writer % 3 == 1 {
                        request_stop(LAB).await;
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
        assert!(!leases().contains_key(LAB));
    }
}
