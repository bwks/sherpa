//! The limit on failed logins, per username and source address together, per address and per
//! username alone, that every login route goes through.
//!
//! Within [`FAILURE_WINDOW`] of a key's first counted failure:
//! - [`PAIR_FAILURE_LIMIT`] failures for one username from one address lock that pair only, so
//!   someone who knows a username cannot lock its user out from anywhere else;
//! - [`ADDRESS_FAILURE_LIMIT`] failures from one address lock the address for every username;
//! - [`USERNAME_FAILURE_LIMIT`] failures for one username, from any addresses, lock the username,
//!   which slows a guess spread over many addresses.
//!
//! A locked key is refused for [`LOCKOUT`]. A successful login clears only its own pair, so a
//! pair locked from another address stays locked and the username-alone count keeps counting
//! until its window ends. Unknown usernames are counted like known ones, so a lockout says
//! nothing about which usernames exist.
//!
//! An attempt is counted when it starts, not when it fails, so logins made at the same moment
//! cannot check more passwords than the limit allows. The count is given back when the attempt
//! ends without an answer, such as when the database cannot be read.
//!
//! An IPv4 address is counted alone. An IPv6 address is counted by its /64 prefix, the smallest
//! block a host is usually given, so changing the low 64 bits does not give a client a new count.
//!
//! The counts and locks are kept only in the database's `login_limit_pair`, `login_limit_address`
//! and `login_limit_username` tables, so they outlive a restart. Each step that reads and then
//! changes them runs in one transaction under the database's write lock (`db::login_limit`).
//! [`run_sweeper`] deletes the rows that limit nothing any more. Each table holds at most
//! [`MAX_TRACKED`] rows. A new key of a full table makes room by deleting expired rows, then the
//! oldest row that is not locked; a lock is never deleted. When every row is a lock, the new key is
//! not counted in that table until one expires. A full table never refuses a login. No password,
//! nor anything made from one, is stored.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use db::{Database, LoginAddress, LoginExpiry, LoginKeys, LoginRelease, LoginSubject};
use jiff::Timestamp;
use shared::util::normalize_username;
use tokio::time::{self, MissedTickBehavior};
use tokio_util::sync::CancellationToken;
use tracing::instrument;

/// Failed logins for one username from one address, within [`FAILURE_WINDOW`], that lock the
/// pair.
pub const PAIR_FAILURE_LIMIT: u32 = 5;
/// Failed logins from one address, within [`FAILURE_WINDOW`], that lock it.
pub const ADDRESS_FAILURE_LIMIT: u32 = 20;
/// Failed logins for one username from any addresses, within [`FAILURE_WINDOW`], that lock it.
pub const USERNAME_FAILURE_LIMIT: u32 = 50;
/// How long after its first failure a key's failures still count.
pub const FAILURE_WINDOW: Duration = Duration::from_mins(15);
/// How long a locked key is refused.
pub const LOCKOUT: Duration = Duration::from_mins(15);
/// The most keys of each kind, pairs, addresses and usernames, the limiter keeps counts for: the
/// cap on each table.
pub const MAX_TRACKED: usize = 100_000;
/// The characters of a normalised username the limiter keys on. Longer names cannot be
/// accounts, so a request cannot make one row take more space than this.
const MAX_USERNAME_KEY_CHARS: usize = 64;
/// The failures that lock the pair, the address and the username.
const LIMITS: LoginKeys<u32> = LoginKeys {
    pair: PAIR_FAILURE_LIMIT,
    address: ADDRESS_FAILURE_LIMIT,
    username: USERNAME_FAILURE_LIMIT,
};

/// Where the limiter reads the time from, so tests can move it.
pub(crate) trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// The limit on failed logins for every login route, over the database's login limit tables.
/// Cloning it shares its database.
#[derive(Clone)]
pub struct LoginLimiter {
    db: Database,
    clock: Arc<dyn Clock>,
    /// The most keys of each kind the limiter keeps counts for: [`MAX_TRACKED`].
    max_tracked: usize,
}

/// A login attempt the limiter let through. Report how it ended with [`Attempt::succeeded`],
/// [`Attempt::failed`] or [`Attempt::unanswered`]; dropped without any, its count is given back
/// in the background.
#[must_use]
pub(crate) struct Attempt {
    limiter: LoginLimiter,
    /// The normalised username and the address as it is counted, an IPv4 address or an IPv6
    /// /64 prefix.
    subject: LoginSubject,
    /// The peer's own address.
    peer: IpAddr,
    /// Which of the pair, the address and the username counted this attempt: a table full of
    /// locks may not have.
    counted: LoginKeys<bool>,
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
    /// A limiter over the counts in `db`, reading the time of the real system.
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self::with_clock(db, Arc::new(SystemClock))
    }

    /// A limiter over the counts in `db` that reads the time from `clock`.
    pub(crate) fn with_clock(db: Database, clock: Arc<dyn Clock>) -> Self {
        Self {
            db,
            clock,
            max_tracked: MAX_TRACKED,
        }
    }

    /// This limiter keeping counts for at most `max_tracked` keys of each kind.
    #[cfg(test)]
    fn with_max_tracked(mut self, max_tracked: usize) -> Self {
        self.max_tracked = max_tracked;
        self
    }

    /// Start a login for `username` from `address`, counting it. `None` when its pair, its
    /// address or its username is locked, or has as many attempts counted as its limit: the
    /// login is refused without its password being checked.
    ///
    /// # Errors
    ///
    /// Fails when the counts cannot be read or written; nothing is counted then.
    #[instrument(level = "debug", skip(self), fields(%username, %address))]
    pub(crate) async fn begin(&self, username: &str, address: IpAddr) -> Result<Option<Attempt>> {
        let peer = address.to_canonical();
        let subject = LoginSubject {
            username: username_key(username),
            address: LoginAddress::of(peer),
        };
        let counted =
            db::begin_login_attempt(&self.db, &subject, LIMITS, self.expiry()?, self.max_tracked)
                .await
                .context("Failed to count the login attempt")?;
        let Some(counted) = counted else {
            return Ok(None);
        };
        Ok(Some(Attempt {
            limiter: self.clone(),
            subject,
            peer,
            counted,
            ended: false,
        }))
    }

    /// The moment now, and so which rows have expired.
    fn expiry(&self) -> Result<LoginExpiry> {
        expiry_at(self.clock.now())
    }
}

impl Attempt {
    /// The password was right: clear this pair's count, and give back the address's and the
    /// username's, so the attempt is counted nowhere.
    ///
    /// # Errors
    ///
    /// Fails when the counts cannot be written; they are left as they were.
    pub(crate) async fn succeeded(mut self) -> Result<()> {
        self.ended = true;
        let releases = LoginKeys {
            pair: LoginRelease::Clear,
            ..self.counted.map(LoginRelease::give_back_if)
        };
        db::give_back_login_attempt(&self.limiter.db, &self.subject, releases)
            .await
            .context("Failed to clear the login count")
    }

    /// The username is unknown or the password was wrong: keep the counts, and lock the pair,
    /// the address or the username that has reached its limit.
    ///
    /// # Errors
    ///
    /// Fails when the locks cannot be written. The counts stay, so a key at its limit is still
    /// refused until its window ends.
    pub(crate) async fn failed(mut self) -> Result<()> {
        self.ended = true;
        let locked_until = self
            .limiter
            .clock
            .now()
            .checked_add(LOCKOUT)
            .context("The lockout ends out of range")?;
        let locked =
            db::lock_full_login_keys(&self.limiter.db, &self.subject, LIMITS, locked_until)
                .await
                .context("Failed to lock the login")?;
        if locked.pair {
            tracing::warn!(
                username = %self.subject.username,
                address = %self.subject.address,
                failures = PAIR_FAILURE_LIMIT,
                lockout_seconds = LOCKOUT.as_secs(),
                "Login locked for this username from this address: too many failed logins"
            );
        }
        if locked.address {
            tracing::warn!(
                username = %self.subject.username,
                address = %self.subject.address,
                failures = ADDRESS_FAILURE_LIMIT,
                lockout_seconds = LOCKOUT.as_secs(),
                "Login locked for this address: too many failed logins"
            );
        }
        if locked.username {
            tracing::warn!(
                username = %self.subject.username,
                address = %self.peer,
                failures = USERNAME_FAILURE_LIMIT,
                lockout_seconds = LOCKOUT.as_secs(),
                "Login locked for this username from every address: too many failed logins"
            );
        }
        Ok(())
    }

    /// The attempt ended without an answer, such as when the database could not be read: give
    /// its count back.
    ///
    /// # Errors
    ///
    /// Fails when the counts cannot be written; the attempt then stays counted.
    pub(crate) async fn unanswered(mut self) -> Result<()> {
        self.ended = true;
        let releases = self.counted.map(LoginRelease::give_back_if);
        db::give_back_login_attempt(&self.limiter.db, &self.subject, releases)
            .await
            .context("Failed to give back the login attempt")
    }
}

impl Drop for Attempt {
    /// An attempt dropped unreported, such as a login whose request was cancelled, gives its
    /// count back in a task of its own, as a drop cannot wait for the database.
    fn drop(&mut self) {
        if self.ended {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::warn!("Login attempt dropped outside a runtime; its count is kept");
            return;
        };
        self.ended = true;
        let db = self.limiter.db.clone();
        let subject = self.subject.clone();
        let releases = self.counted.map(LoginRelease::give_back_if);
        runtime.spawn(async move {
            if let Err(error) = db::give_back_login_attempt(&db, &subject, releases).await {
                tracing::warn!(
                    error = format!("{error:#}"),
                    "Failed to give back a dropped login attempt"
                );
            }
        });
    }
}

/// Delete the login counts and locks that limit nothing any more by `now`: locks that have
/// ended, and counts not locked whose window has passed. Returns how many were deleted.
///
/// # Errors
///
/// Fails when the database cannot be written.
#[instrument(level = "debug", skip(db))]
pub async fn sweep(db: &Database, now: Timestamp) -> Result<usize> {
    db::delete_expired_login_limits(db, expiry_at(now)?)
        .await
        .context("Failed to sweep expired login counts")
}

/// Sweep expired login counts and locks at once, then every `interval`, until `cancel` fires.
///
/// An expired row is already ignored by the limiter; the sweep only keeps the table from
/// holding rows that limit nothing. A failed sweep is logged and retried at the next interval.
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
                    "Deleted expired login counts"
                ),
                Err(error) => tracing::warn!(
                    error = format!("{error:#}"),
                    "Login limit sweep failed"
                ),
            },
        }
    }
}

/// Which rows have expired at `now`: locks ended by then, and counts not locked whose window
/// started [`FAILURE_WINDOW`] or more before.
fn expiry_at(now: Timestamp) -> Result<LoginExpiry> {
    Ok(LoginExpiry {
        now,
        window_started_by: now
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
    /// The cap on keys of each kind the cases about a full table use in place of
    /// [`MAX_TRACKED`], so they fill it in seconds.
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
                .failed()
                .await
                .expect("failed");
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

    /// Whether the limiter lets an attempt through now. The attempt is given back.
    async fn admits(limiter: &LoginLimiter, username: &str, address: IpAddr) -> bool {
        match limiter.begin(username, address).await.expect("begin") {
            Some(attempt) => {
                attempt.unanswered().await.expect("unanswered");
                true
            }
            None => false,
        }
    }

    /// How many pairs, addresses and usernames have counts kept.
    async fn tracked(limiter: &LoginLimiter) -> LoginKeys<usize> {
        db::count_login_limits(&limiter.db).await.expect("count")
    }

    /// The same count of keys kept for pairs, addresses and usernames.
    fn each(keys: usize) -> LoginKeys<usize> {
        LoginKeys {
            pair: keys,
            address: keys,
            username: keys,
        }
    }

    /// The address of the `n`th distinct client in a test, 10.x.y.z.
    fn nth_address(n: usize) -> IpAddr {
        let n = u32::try_from(n).expect("small n");
        IpAddr::V4(Ipv4Addr::from(0x0a00_0000 | n))
    }

    #[tokio::test]
    async fn user_login_019_a_locked_pair_is_let_in_again_15_minutes_after_it_was_locked() {
        let (limiter, clock) = limiter().await;
        fail(&limiter, "alice", ADDRESS, 5).await;
        assert!(!admits(&limiter, "alice", ADDRESS).await);

        clock.advance(Duration::from_secs(15 * 60 - 1));
        assert!(!admits(&limiter, "alice", ADDRESS).await);

        clock.advance(Duration::from_secs(1));
        assert!(admits(&limiter, "alice", ADDRESS).await);
    }

    #[tokio::test]
    async fn user_login_020_a_locked_address_is_let_in_again_15_minutes_after_it_was_locked() {
        let (limiter, clock) = limiter().await;
        for n in 0..20 {
            fail(&limiter, &format!("user{n}"), ADDRESS, 1).await;
        }
        assert!(!admits(&limiter, "bob", ADDRESS).await);

        clock.advance(Duration::from_secs(15 * 60 - 1));
        assert!(!admits(&limiter, "bob", ADDRESS).await);

        clock.advance(Duration::from_secs(1));
        assert!(admits(&limiter, "bob", ADDRESS).await);
    }

    #[tokio::test]
    async fn user_login_021_failures_15_minutes_after_the_first_start_a_new_count() {
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
        succeed(&limiter, "alice", ADDRESS).await;

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
        for attempt in in_flight.into_iter().flatten() {
            attempt.unanswered().await.expect("unanswered");
        }
    }

    #[tokio::test]
    async fn user_login_027_an_attempt_that_ends_without_an_answer_is_not_counted() {
        let (limiter, _clock) = limiter().await;
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
        // Dropped unreported, as when its request is cancelled: given back in the background.
        for _ in 0..5 {
            drop(
                limiter
                    .begin("alice", ADDRESS)
                    .await
                    .expect("begin")
                    .expect("attempt let through"),
            );
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while tracked(&limiter).await != each(0) {
            assert!(
                Instant::now() < deadline,
                "dropped attempts were not given back"
            );
            sleep(Duration::from_millis(10)).await;
        }

        fail(&limiter, "alice", ADDRESS, 4).await;
        assert!(admits(&limiter, "alice", ADDRESS).await);
    }

    #[tokio::test]
    async fn user_login_028_counts_are_kept_for_no_more_than_the_cap_of_keys_in_each_table() {
        let (limiter, _clock) = limiter().await;
        let limiter = limiter.with_max_tracked(TEST_CAP);
        for n in 0..TEST_CAP + 10 {
            fail(&limiter, &format!("user{n}"), nth_address(n), 1).await;
        }
        assert_eq!(tracked(&limiter).await, each(TEST_CAP));
    }

    #[tokio::test]
    async fn user_login_029_when_the_counts_are_full_a_lock_is_kept_over_a_count_that_is_not_one() {
        let (limiter, _clock) = limiter().await;
        let limiter = limiter.with_max_tracked(TEST_CAP);
        fail(&limiter, "alice", ADDRESS, 5).await;
        for n in 0..TEST_CAP + 10 {
            fail(&limiter, &format!("user{n}"), nth_address(n), 1).await;
        }
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

    #[tokio::test]
    async fn user_login_034_a_table_full_of_locks_drops_none_and_still_lets_an_unlocked_user_log_in()
     {
        let (limiter, _clock) = limiter().await;
        let limiter = limiter.with_max_tracked(TEST_CAP);
        for n in 0..TEST_CAP {
            fail(&limiter, &format!("user{n}"), nth_address(n), 5).await;
        }
        assert_eq!(tracked(&limiter).await.pair, TEST_CAP);

        let attempt = limiter
            .begin("alice", ADDRESS)
            .await
            .expect("begin")
            .expect("attempt let through");
        assert_eq!(tracked(&limiter).await.pair, TEST_CAP);
        attempt.succeeded().await.expect("succeeded");
        for n in 0..TEST_CAP {
            assert!(!admits(&limiter, &format!("user{n}"), nth_address(n)).await);
        }
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
    async fn user_login_037_the_sweep_deletes_expired_counts_and_locks_and_never_a_live_lock() {
        let db = database().await;
        let now = Timestamp::now();
        let past = ManualClock::at(now.checked_sub(Duration::from_mins(31)).unwrap());
        let old = LoginLimiter::with_clock(db.clone(), past.clone());
        // An ended lock and a count whose window has passed.
        fail(&old, "alice", ADDRESS, 5).await;
        past.advance(Duration::from_mins(1));
        fail(&old, "bob", OTHER_ADDRESS, 1).await;
        let live = LoginLimiter::with_clock(db.clone(), ManualClock::at(now));
        fail(&live, "carol", nth_address(1), 5).await;
        assert_eq!(tracked(&live).await, each(3));

        let cancel = CancellationToken::new();
        let sweeper = tokio::spawn(run_sweeper(
            db.clone(),
            cancel.clone(),
            Duration::from_secs(3600),
        ));
        let deadline = Instant::now() + Duration::from_secs(10);
        while tracked(&live).await != each(1) {
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
}
