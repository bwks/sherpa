//! The limit on failed logins, per username and source address together, per address and per
//! username alone, that every login route goes through.
//!
//! Counted over the failures of the last [`FAILURE_WINDOW`], a sliding window, a login is refused
//! while:
//! - its username and address together have [`PAIR_FAILURE_LIMIT`] failures, which refuses that
//!   pair only, so someone who knows a username cannot lock its user out from anywhere else;
//! - its address has [`ADDRESS_FAILURE_LIMIT`], for every username;
//! - its username has [`USERNAME_FAILURE_LIMIT`], from any addresses, which slows a guess spread
//!   over many addresses.
//!
//! No lock is stored: a lock ends when enough of its failures are older than the window. A
//! refused login writes no failure, so attempts during a lock do not lengthen it. A successful
//! login deletes only its own pair's failures, so a pair locked from another address stays locked
//! and the username's failures from other addresses keep counting. Unknown usernames are counted
//! like known ones, so a lockout says nothing about which usernames exist.
//!
//! An admitted attempt writes its failure when it starts, not when it fails, so logins made at the
//! same moment cannot check more passwords than the limit allows. The failure stays when the
//! password is wrong, and is deleted when the attempt ends without an answer, such as when the
//! database cannot be read or the request is dropped.
//!
//! An IPv4 address is counted alone. An IPv6 address is counted by its /64 prefix, the smallest
//! block a host is usually given, so changing the low 64 bits does not give a client a new count.
//!
//! The failures are kept only in the database's `login_failure` table, so a lock outlives a
//! restart. [`run_sweeper`] deletes those older than the window. The table holds at most
//! [`MAX_TRACKED`] rows. At the cap an attempt first deletes the expired rows, and when that frees
//! nothing it is admitted or refused on its counts as usual but writes no failure; a live row is
//! never deleted, and a full table never refuses a login. No password, nor anything made from
//! one, is stored.

use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use db::{
    Database, LoginAddress, LoginAdmission, LoginFailureId, LoginKeys, LoginSubject, LoginWindow,
};
use jiff::Timestamp;
use shared::util::normalize_username;
use tokio::time::{self, MissedTickBehavior};
use tokio_util::sync::CancellationToken;
use tracing::instrument;

/// Failed logins for one username from one address, within [`FAILURE_WINDOW`], that refuse the
/// pair.
pub const PAIR_FAILURE_LIMIT: u32 = 5;
/// Failed logins from one address, within [`FAILURE_WINDOW`], that refuse it.
pub const ADDRESS_FAILURE_LIMIT: u32 = 20;
/// Failed logins for one username from any addresses, within [`FAILURE_WINDOW`], that refuse it.
pub const USERNAME_FAILURE_LIMIT: u32 = 50;
/// How long a failed login counts toward the limits.
pub const FAILURE_WINDOW: Duration = Duration::from_mins(15);
/// The most failed logins the limiter keeps: the cap on the `login_failure` table.
pub const MAX_TRACKED: usize = 100_000;
/// The characters of a normalised username the limiter keys on. Longer names cannot be
/// accounts, so a request cannot make one row take more space than this.
const MAX_USERNAME_KEY_CHARS: usize = 64;
/// The failures that refuse the pair, the address and the username.
const LIMITS: LoginKeys<u32> = LoginKeys {
    pair: PAIR_FAILURE_LIMIT,
    address: ADDRESS_FAILURE_LIMIT,
    username: USERNAME_FAILURE_LIMIT,
};

/// Where the limiter reads the time from, so tests can move it.
pub(crate) trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// The limit on failed logins for every login route, over the database's `login_failure` table.
/// Cloning it shares its database.
#[derive(Clone)]
pub struct LoginLimiter {
    db: Database,
    clock: Arc<dyn Clock>,
    /// The most failed logins the limiter keeps: [`MAX_TRACKED`].
    max_tracked: usize,
    /// Whether the table being full has been logged since an attempt last wrote its failure.
    full_logged: Arc<AtomicBool>,
}

/// A login attempt the limiter let through. Report how it ended with [`Attempt::succeeded`],
/// [`Attempt::failed`] or [`Attempt::unanswered`]; dropped without any, its failure is deleted
/// in the background.
#[must_use]
pub(crate) struct Attempt {
    limiter: LoginLimiter,
    /// The normalised username and the address as it is counted, an IPv4 address or an IPv6
    /// /64 prefix.
    subject: LoginSubject,
    /// The peer's own address.
    peer: IpAddr,
    /// The failures in the window for the pair, the address and the username, this attempt's
    /// own included when it was written.
    counts: LoginKeys<u32>,
    /// The failure this attempt wrote when it started, `None` when the table was full.
    failure: Option<LoginFailureId>,
    ended: bool,
}

/// The time of the real system.
struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

impl LoginLimiter {
    /// A limiter over the failures in `db`, reading the time of the real system.
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self::with_clock(db, Arc::new(SystemClock))
    }

    /// A limiter over the failures in `db` that reads the time from `clock`.
    pub(crate) fn with_clock(db: Database, clock: Arc<dyn Clock>) -> Self {
        Self {
            db,
            clock,
            max_tracked: MAX_TRACKED,
            full_logged: Arc::new(AtomicBool::new(false)),
        }
    }

    /// This limiter keeping at most `max_tracked` failed logins.
    #[cfg(test)]
    fn with_max_tracked(mut self, max_tracked: usize) -> Self {
        self.max_tracked = max_tracked;
        self
    }

    /// Start a login for `username` from `address`, writing its failure. `None` when its pair,
    /// its address or its username has as many failures in the window as its limit: the login
    /// is refused without its password being checked, and nothing is written.
    ///
    /// # Errors
    ///
    /// Fails when the failures cannot be read or written; nothing is written then.
    #[instrument(level = "debug", skip(self), fields(%username, %address))]
    pub(crate) async fn begin(&self, username: &str, address: IpAddr) -> Result<Option<Attempt>> {
        let peer = address.to_canonical();
        let subject = LoginSubject {
            username: username_key(username),
            address: LoginAddress::of(peer),
        };
        let admission = db::begin_login_attempt(
            &self.db,
            &subject,
            LIMITS,
            window_at(self.clock.now())?,
            self.max_tracked,
        )
        .await
        .context("Failed to count the login attempt")?;
        let LoginAdmission::Admitted { counts, failure } = admission else {
            return Ok(None);
        };
        if failure.is_some() {
            self.full_logged.store(false, Ordering::Relaxed);
        } else if !self.full_logged.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                max_tracked = self.max_tracked,
                "Login failures are at their cap: logins are let through or refused on the \
                 failures kept, and new failures are not counted until old ones expire"
            );
        }
        Ok(Some(Attempt {
            limiter: self.clone(),
            subject,
            peer,
            counts,
            failure,
            ended: false,
        }))
    }
}

impl Attempt {
    /// The password was right: delete every failure of this username and address together,
    /// this attempt's own included, and no other.
    ///
    /// # Errors
    ///
    /// Fails when the failures cannot be deleted; they are left as they were.
    pub(crate) async fn succeeded(mut self) -> Result<()> {
        self.ended = true;
        db::clear_login_failures(&self.limiter.db, &self.subject)
            .await
            .context("Failed to clear the failed logins")
    }

    /// The username is unknown or the password was wrong: keep the failure, and log each limit
    /// it brings to its count.
    pub(crate) fn failed(mut self) {
        self.ended = true;
        if self.failure.is_none() {
            return;
        }
        if self.counts.pair == PAIR_FAILURE_LIMIT {
            tracing::warn!(
                username = %self.subject.username,
                address = %self.subject.address,
                failures = PAIR_FAILURE_LIMIT,
                window_seconds = FAILURE_WINDOW.as_secs(),
                "Login locked for this username from this address: too many failed logins"
            );
        }
        if self.counts.address == ADDRESS_FAILURE_LIMIT {
            tracing::warn!(
                username = %self.subject.username,
                address = %self.subject.address,
                failures = ADDRESS_FAILURE_LIMIT,
                window_seconds = FAILURE_WINDOW.as_secs(),
                "Login locked for this address: too many failed logins"
            );
        }
        if self.counts.username == USERNAME_FAILURE_LIMIT {
            tracing::warn!(
                username = %self.subject.username,
                address = %self.peer,
                failures = USERNAME_FAILURE_LIMIT,
                window_seconds = FAILURE_WINDOW.as_secs(),
                "Login locked for this username from every address: too many failed logins"
            );
        }
    }

    /// The attempt ended without an answer, such as when the database could not be read:
    /// delete its own failure.
    ///
    /// # Errors
    ///
    /// Fails when the failure cannot be deleted; it then counts until it expires.
    pub(crate) async fn unanswered(mut self) -> Result<()> {
        self.ended = true;
        let Some(failure) = self.failure else {
            return Ok(());
        };
        db::delete_login_failure(&self.limiter.db, &self.subject, failure)
            .await
            .context("Failed to delete the unanswered login attempt")
    }
}

impl Drop for Attempt {
    /// An attempt dropped unreported, such as a login whose request was cancelled, deletes its
    /// failure in a task of its own, as a drop cannot wait for the database.
    fn drop(&mut self) {
        if self.ended {
            return;
        }
        self.ended = true;
        let Some(failure) = self.failure else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::warn!("Login attempt dropped outside a runtime; its failure is kept");
            return;
        };
        let db = self.limiter.db.clone();
        let subject = self.subject.clone();
        runtime.spawn(async move {
            if let Err(error) = db::delete_login_failure(&db, &subject, failure).await {
                tracing::warn!(
                    error = format!("{error:#}"),
                    "Failed to delete a dropped login attempt"
                );
            }
        });
    }
}

/// Delete the failed logins older than [`FAILURE_WINDOW`] at `now`, which count toward no limit
/// any more. Returns how many were deleted.
///
/// # Errors
///
/// Fails when the database cannot be written.
#[instrument(level = "debug", skip(db))]
pub async fn sweep(db: &Database, now: Timestamp) -> Result<usize> {
    db::delete_expired_login_failures(db, window_at(now)?.expired_by)
        .await
        .context("Failed to sweep expired failed logins")
}

/// Sweep expired failed logins at once, then every `interval`, until `cancel` fires.
///
/// An expired failure is already not counted; the sweep only keeps the table from holding rows
/// that limit nothing. A failed sweep is logged and retried at the next interval.
#[instrument(skip(db, cancel))]
pub async fn run_sweeper(db: Database, cancel: CancellationToken, interval: Duration) {
    let mut ticks = time::interval(interval);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                tracing::info!("Login limit sweeper stopped");
                return;
            }
            _ = ticks.tick() => match sweep(&db, Timestamp::now()).await {
                Ok(0) => {}
                Ok(swept) => tracing::info!(
                    event = "auth.login_limits_swept",
                    swept,
                    "Deleted expired failed logins"
                ),
                Err(error) => tracing::warn!(
                    error = format!("{error:#}"),
                    "Login limit sweep failed"
                ),
            },
        }
    }
}

/// The window at `now`: failures made after [`FAILURE_WINDOW`] before it count.
fn window_at(now: Timestamp) -> Result<LoginWindow> {
    Ok(LoginWindow {
        now,
        expired_by: now
            .checked_sub(FAILURE_WINDOW)
            .context("The login window starts out of range")?,
    })
}

/// The key a username is counted under: as accounts are looked up, and no longer than any
/// account's name can be.
fn username_key(username: &str) -> String {
    normalize_username(username)
        .chars()
        .take(MAX_USERNAME_KEY_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::sync::Mutex;

    use tokio::runtime::Runtime;
    use tokio::time::{Instant, sleep, timeout};

    use super::*;
    use crate::log_capture::capture_log;

    const ADDRESS: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10));
    const OTHER_ADDRESS: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 20));
    /// The cap on failed logins the cases about a full table use in place of [`MAX_TRACKED`],
    /// so they fill it in seconds.
    const TEST_CAP: usize = 50;
    /// The time a test's clock starts at, in Unix seconds.
    const START: i64 = 1_800_000_000;

    /// A clock that moves only when a test moves it.
    struct ManualClock(Mutex<Timestamp>);

    impl Clock for ManualClock {
        fn now(&self) -> Timestamp {
            *self.0.lock().unwrap()
        }
    }

    impl ManualClock {
        fn at(now: Timestamp) -> Arc<Self> {
            Arc::new(Self(Mutex::new(now)))
        }

        fn advance(&self, by: Duration) {
            let mut now = self.0.lock().unwrap();
            *now = now.checked_add(by).unwrap();
        }
    }

    async fn database() -> Database {
        let db = db::open_in_memory().await.expect("open database");
        db::apply_schema(&db).await.expect("apply schema");
        db
    }

    async fn limiter() -> (LoginLimiter, Arc<ManualClock>) {
        let clock = ManualClock::at(Timestamp::from_second(START).unwrap());
        (
            LoginLimiter::with_clock(database().await, clock.clone()),
            clock,
        )
    }

    /// A runtime on this thread, for tests whose log is captured on it.
    fn runtime() -> Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    async fn fail(limiter: &LoginLimiter, username: &str, address: IpAddr, times: usize) {
        for _ in 0..times {
            limiter
                .begin(username, address)
                .await
                .expect("begin")
                .expect("attempt let through")
                .failed();
        }
    }

    async fn succeed(limiter: &LoginLimiter, username: &str, address: IpAddr) {
        limiter
            .begin(username, address)
            .await
            .expect("begin")
            .expect("attempt let through")
            .succeeded()
            .await
            .expect("succeeded");
    }

    /// Whether the limiter lets an attempt through now. The attempt's failure is deleted.
    async fn admits(limiter: &LoginLimiter, username: &str, address: IpAddr) -> bool {
        match limiter.begin(username, address).await.expect("begin") {
            Some(attempt) => {
                attempt.unanswered().await.expect("unanswered");
                true
            }
            None => false,
        }
    }

    /// How many failed logins the table holds.
    async fn rows(limiter: &LoginLimiter) -> usize {
        db::count_login_failures(&limiter.db).await.expect("count")
    }

    /// The address of the `n`th distinct client in a test, 10.x.y.z.
    fn nth_address(n: usize) -> IpAddr {
        let n = u32::try_from(n).expect("small n");
        IpAddr::V4(Ipv4Addr::from(0x0a00_0000 | n))
    }

    #[tokio::test]
    async fn user_login_019_a_locked_pair_is_let_in_again_once_its_counted_failures_are_15_minutes_old()
     {
        let (limiter, clock) = limiter().await;
        fail(&limiter, "alice", ADDRESS, 4).await;
        clock.advance(Duration::from_mins(10));
        fail(&limiter, "alice", ADDRESS, 1).await;
        assert!(!admits(&limiter, "alice", ADDRESS).await);

        // 5 minutes after the lock, the first 4 failures are 15 minutes old.
        clock.advance(Duration::from_secs(5 * 60 - 1));
        assert!(!admits(&limiter, "alice", ADDRESS).await);
        clock.advance(Duration::from_secs(1));
        assert!(admits(&limiter, "alice", ADDRESS).await);
    }

    #[tokio::test]
    async fn user_login_020_a_locked_address_is_let_in_again_once_its_counted_failures_are_15_minutes_old()
     {
        let (limiter, clock) = limiter().await;
        for n in 0..16 {
            fail(&limiter, &format!("user{n}"), ADDRESS, 1).await;
        }
        clock.advance(Duration::from_mins(10));
        for n in 16..20 {
            fail(&limiter, &format!("user{n}"), ADDRESS, 1).await;
        }
        assert!(!admits(&limiter, "bob", ADDRESS).await);

        clock.advance(Duration::from_secs(5 * 60 - 1));
        assert!(!admits(&limiter, "bob", ADDRESS).await);
        clock.advance(Duration::from_secs(1));
        assert!(admits(&limiter, "bob", ADDRESS).await);
    }

    #[tokio::test]
    async fn user_login_021_a_failure_stops_counting_15_minutes_after_it_was_made() {
        let (limiter, clock) = limiter().await;
        fail(&limiter, "alice", ADDRESS, 4).await;
        clock.advance(Duration::from_mins(15));
        fail(&limiter, "alice", ADDRESS, 4).await;
        assert!(admits(&limiter, "alice", ADDRESS).await);
    }

    #[tokio::test]
    async fn user_login_022_a_successful_login_clears_its_own_pair_and_no_other() {
        let (limiter, _clock) = limiter().await;
        fail(&limiter, "alice", ADDRESS, 4).await;
        fail(&limiter, "alice", OTHER_ADDRESS, 4).await;
        fail(&limiter, "bob", ADDRESS, 4).await;
        succeed(&limiter, "alice", ADDRESS).await;
        assert_eq!(rows(&limiter).await, 8);

        fail(&limiter, "alice", ADDRESS, 4).await;
        assert!(admits(&limiter, "alice", ADDRESS).await);
        fail(&limiter, "alice", OTHER_ADDRESS, 1).await;
        assert!(!admits(&limiter, "alice", OTHER_ADDRESS).await);
    }

    #[tokio::test]
    async fn user_login_023_a_successful_login_does_not_count_against_its_address() {
        let (limiter, _clock) = limiter().await;
        for n in 0..19 {
            fail(&limiter, &format!("user{n}"), ADDRESS, 1).await;
        }
        for _ in 0..5 {
            succeed(&limiter, "alice", ADDRESS).await;
        }
        assert!(admits(&limiter, "bob", ADDRESS).await);
    }

    #[tokio::test]
    async fn user_login_024_a_username_is_counted_in_any_case_and_with_surrounding_space() {
        let (limiter, _clock) = limiter().await;
        for username in ["alice", "Alice", " ALICE", "alice ", "aLiCe"] {
            fail(&limiter, username, ADDRESS, 1).await;
        }
        assert!(!admits(&limiter, "alice", ADDRESS).await);
    }

    #[tokio::test]
    async fn user_login_025_an_ipv4_address_counts_alike_as_ipv4_and_as_ipv4_mapped_ipv6() {
        let (limiter, _clock) = limiter().await;
        let mapped = IpAddr::V6(Ipv4Addr::new(192, 0, 2, 10).to_ipv6_mapped());
        for n in 0..20 {
            fail(&limiter, &format!("user{n}"), mapped, 1).await;
        }
        assert!(!admits(&limiter, "bob", ADDRESS).await);
    }

    #[tokio::test]
    async fn user_login_026_attempts_made_at_once_check_no_more_passwords_than_the_limit() {
        let (limiter, _clock) = limiter().await;
        let attempts = (0..6).map(|_| limiter.begin("alice", ADDRESS));
        let started = futures_util::future::join_all(attempts).await;
        let in_flight: Vec<_> = started
            .into_iter()
            .map(|started| started.expect("begin"))
            .collect();
        assert_eq!(
            in_flight.iter().filter(|attempt| attempt.is_some()).count(),
            5
        );
        assert_eq!(rows(&limiter).await, 5);
        for attempt in in_flight.into_iter().flatten() {
            attempt.unanswered().await.expect("unanswered");
        }
        assert_eq!(rows(&limiter).await, 0);
    }

    #[tokio::test]
    async fn user_login_027_an_attempt_that_ends_without_an_answer_is_not_counted() {
        let (limiter, _clock) = limiter().await;
        fail(&limiter, "alice", ADDRESS, 2).await;
        for _ in 0..5 {
            limiter
                .begin("alice", ADDRESS)
                .await
                .expect("begin")
                .expect("attempt let through")
                .unanswered()
                .await
                .expect("unanswered");
        }
        // Dropped unreported, as when its request is cancelled: deleted in the background, so
        // no more than the 3 the pair's limit leaves room for start before they are.
        for _ in 0..3 {
            drop(
                limiter
                    .begin("alice", ADDRESS)
                    .await
                    .expect("begin")
                    .expect("attempt let through"),
            );
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while rows(&limiter).await != 2 {
            assert!(
                Instant::now() < deadline,
                "dropped attempts were not deleted"
            );
            sleep(Duration::from_millis(10)).await;
        }

        fail(&limiter, "alice", ADDRESS, 2).await;
        assert!(admits(&limiter, "alice", ADDRESS).await);
        fail(&limiter, "alice", ADDRESS, 1).await;
        assert!(!admits(&limiter, "alice", ADDRESS).await);
    }

    #[tokio::test]
    async fn user_login_028_the_table_holds_no_more_failed_logins_than_its_cap() {
        let (limiter, _clock) = limiter().await;
        let limiter = limiter.with_max_tracked(TEST_CAP);
        for n in 0..TEST_CAP + 10 {
            fail(&limiter, &format!("user{n}"), nth_address(n), 1).await;
        }
        assert_eq!(rows(&limiter).await, TEST_CAP);
    }

    #[tokio::test]
    async fn user_login_029_at_the_cap_expired_failures_make_room_and_a_live_one_never_does() {
        let (limiter, clock) = limiter().await;
        let limiter = limiter.with_max_tracked(TEST_CAP);
        for n in 0..TEST_CAP - 5 {
            fail(&limiter, &format!("user{n}"), nth_address(n), 1).await;
        }
        clock.advance(Duration::from_mins(5));
        fail(&limiter, "alice", ADDRESS, 5).await;
        fail(&limiter, "bob", OTHER_ADDRESS, 1).await;
        assert_eq!(rows(&limiter).await, TEST_CAP);
        assert!(!admits(&limiter, "alice", ADDRESS).await);

        // The first failures are now expired, and alice's 5 are not.
        clock.advance(Duration::from_mins(10));
        fail(&limiter, "bob", OTHER_ADDRESS, 1).await;
        assert_eq!(rows(&limiter).await, 6);
        assert!(!admits(&limiter, "alice", ADDRESS).await);
    }

    #[test]
    fn user_login_030_a_lockout_is_logged_once_at_warn_with_the_username_and_address() {
        let runtime = runtime();
        let ((), log) = capture_log(|| {
            runtime.block_on(async {
                let (limiter, _clock) = limiter().await;
                fail(&limiter, "alice", ADDRESS, 5).await;
                for _ in 0..3 {
                    assert!(!admits(&limiter, "alice", ADDRESS).await);
                }
            });
        });
        let lockouts: Vec<_> = log
            .lines()
            .filter(|line| line.contains("Login locked"))
            .collect();
        assert_eq!(lockouts.len(), 1, "{log}");
        assert!(lockouts[0].contains("WARN"), "{log}");
        assert!(lockouts[0].contains("username=alice"), "{log}");
        assert!(lockouts[0].contains("address=192.0.2.10"), "{log}");
    }

    #[test]
    fn user_login_031_ipv6_addresses_in_one_64_prefix_share_one_count_and_lock() {
        let first = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 1, 2, 0, 0, 0, 1));
        let same_prefix = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 1, 2, 0xffff, 0xabcd, 9, 9));
        let other_prefix = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 1, 3, 0, 0, 0, 1));
        let runtime = runtime();
        let (limiter, log) = capture_log(|| {
            runtime.block_on(async {
                let (limiter, _clock) = limiter().await;
                for n in 0..10 {
                    fail(&limiter, &format!("user{n}"), first, 1).await;
                }
                for n in 10..20 {
                    fail(&limiter, &format!("user{n}"), same_prefix, 1).await;
                }
                limiter
            })
        });
        runtime.block_on(async {
            assert!(!admits(&limiter, "bob", first).await);
            assert!(!admits(&limiter, "bob", same_prefix).await);
            assert!(admits(&limiter, "bob", other_prefix).await);
        });
        let lockout = log
            .lines()
            .find(|line| line.contains("Login locked for this address"))
            .unwrap_or_else(|| panic!("no address lockout logged: {log}"));
        assert!(lockout.contains("address=2001:db8:1:2::/64"), "{log}");
    }

    #[tokio::test]
    async fn user_login_032_a_locked_pair_leaves_its_username_free_to_log_in_from_another_address()
    {
        let (limiter, _clock) = limiter().await;
        fail(&limiter, "alice", ADDRESS, 5).await;
        assert!(!admits(&limiter, "alice", ADDRESS).await);
        succeed(&limiter, "alice", OTHER_ADDRESS).await;
    }

    #[tokio::test]
    async fn user_login_033_50_failures_for_a_username_across_addresses_lock_it_from_every_address()
    {
        let (limiter, clock) = limiter().await;
        for n in 0..49 {
            fail(&limiter, "alice", nth_address(n), 1).await;
        }
        assert!(admits(&limiter, "alice", ADDRESS).await);
        fail(&limiter, "alice", nth_address(49), 1).await;
        assert!(!admits(&limiter, "alice", ADDRESS).await);
        assert!(admits(&limiter, "bob", nth_address(0)).await);

        clock.advance(Duration::from_mins(15));
        assert!(admits(&limiter, "alice", ADDRESS).await);
    }

    #[test]
    fn user_login_034_a_full_table_of_live_failures_is_logged_once_and_admits_or_refuses_on_the_counts_kept()
     {
        let runtime = runtime();
        let ((), log) = capture_log(|| {
            runtime.block_on(async {
                let (limiter, _clock) = limiter().await;
                let limiter = limiter.with_max_tracked(TEST_CAP);
                for n in 0..TEST_CAP / 5 {
                    fail(&limiter, &format!("user{n}"), nth_address(n), 5).await;
                }
                assert_eq!(rows(&limiter).await, TEST_CAP);

                for _ in 0..3 {
                    succeed(&limiter, "alice", ADDRESS).await;
                    fail(&limiter, "bob", OTHER_ADDRESS, 1).await;
                }
                assert_eq!(rows(&limiter).await, TEST_CAP);
                for n in 0..TEST_CAP / 5 {
                    assert!(!admits(&limiter, &format!("user{n}"), nth_address(n)).await);
                }
            });
        });
        let full: Vec<_> = log
            .lines()
            .filter(|line| line.contains("at their cap"))
            .collect();
        assert_eq!(full.len(), 1, "{log}");
        assert!(full[0].contains("WARN"), "{log}");
    }

    #[tokio::test]
    async fn user_login_035_a_success_leaves_a_pair_locked_elsewhere_and_the_username_count_unchanged()
     {
        let (limiter, _clock) = limiter().await;
        fail(&limiter, "alice", OTHER_ADDRESS, 5).await;
        succeed(&limiter, "alice", ADDRESS).await;
        assert!(!admits(&limiter, "alice", OTHER_ADDRESS).await);

        // 5 failures counted, so 44 more leave the username one short of its limit of 50.
        for n in 0..44 {
            fail(&limiter, "alice", nth_address(n), 1).await;
        }
        assert!(admits(&limiter, "alice", ADDRESS).await);
        fail(&limiter, "alice", nth_address(44), 1).await;
        assert!(!admits(&limiter, "alice", ADDRESS).await);
    }

    #[tokio::test]
    async fn user_login_036_a_lock_outlives_a_restart_of_the_limiter_over_the_same_database() {
        let (limiter, clock) = limiter().await;
        fail(&limiter, "alice", ADDRESS, 5).await;
        let db = limiter.db.clone();
        drop(limiter);

        let restarted = LoginLimiter::with_clock(db, clock.clone());
        assert!(!admits(&restarted, "alice", ADDRESS).await);
        clock.advance(Duration::from_mins(15));
        assert!(admits(&restarted, "alice", ADDRESS).await);
    }

    #[tokio::test]
    async fn user_login_037_the_sweep_deletes_failures_older_than_15_minutes_and_never_a_live_one()
    {
        let db = database().await;
        let now = Timestamp::now();
        let past = ManualClock::at(now.checked_sub(Duration::from_mins(31)).unwrap());
        let old = LoginLimiter::with_clock(db.clone(), past.clone());
        fail(&old, "alice", ADDRESS, 5).await;
        past.advance(Duration::from_mins(15));
        fail(&old, "bob", OTHER_ADDRESS, 1).await;
        let live = LoginLimiter::with_clock(db.clone(), ManualClock::at(now));
        fail(&live, "carol", nth_address(1), 5).await;
        assert_eq!(rows(&live).await, 11);

        let cancel = CancellationToken::new();
        let sweeper = tokio::spawn(run_sweeper(
            db.clone(),
            cancel.clone(),
            Duration::from_secs(3600),
        ));
        let deadline = Instant::now() + Duration::from_secs(10);
        while rows(&live).await != 5 {
            assert!(
                Instant::now() < deadline,
                "the sweeper did not sweep at start"
            );
            sleep(Duration::from_millis(20)).await;
        }
        cancel.cancel();
        timeout(Duration::from_secs(5), sweeper)
            .await
            .expect("the sweeper did not stop")
            .expect("the sweeper panicked");
        assert!(!admits(&live, "carol", nth_address(1)).await);
    }

    #[tokio::test]
    async fn user_login_038_a_refused_attempt_writes_no_failure_so_a_lock_does_not_lengthen() {
        let (limiter, clock) = limiter().await;
        fail(&limiter, "alice", ADDRESS, 5).await;
        clock.advance(Duration::from_mins(10));
        for _ in 0..3 {
            assert!(!admits(&limiter, "alice", ADDRESS).await);
        }
        assert_eq!(rows(&limiter).await, 5);

        clock.advance(Duration::from_mins(5));
        assert!(admits(&limiter, "alice", ADDRESS).await);
    }
}
