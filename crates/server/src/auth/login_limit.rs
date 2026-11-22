//! The limit on failed logins, per username and per source address, that every login route
//! goes through.
//!
//! A username with [`USERNAME_FAILURE_LIMIT`] failed logins, or an address with
//! [`ADDRESS_FAILURE_LIMIT`], within [`FAILURE_WINDOW`] of its first counted failure is refused
//! for [`LOCKOUT`]. A successful login clears its username's count. Unknown usernames are counted
//! like known ones, so a lockout says nothing about which usernames exist.
//!
//! An attempt is counted when it starts, not when it fails, so logins made at the same moment
//! cannot check more passwords than the limit allows. The count is given back when the attempt
//! ends without an answer, such as when the database cannot be read.
//!
//! Counts are kept in memory only and a restart clears them. Each table holds at most
//! [`MAX_TRACKED`] keys: when it is full, expired entries are dropped first, then the oldest
//! entry that is not locked, then the lock that ends soonest.

use std::collections::HashMap;
use std::hash::Hash;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use shared::util::normalize_username;

/// Failed logins for one username, within [`FAILURE_WINDOW`], that lock it.
pub const USERNAME_FAILURE_LIMIT: u32 = 5;
/// Failed logins from one address, within [`FAILURE_WINDOW`], that lock it.
pub const ADDRESS_FAILURE_LIMIT: u32 = 20;
/// How long after its first failure a username's or an address's failures still count.
pub const FAILURE_WINDOW: Duration = Duration::from_mins(15);
/// How long a locked username or address is refused.
pub const LOCKOUT: Duration = Duration::from_mins(15);
/// The most usernames, and separately the most addresses, the limiter keeps counts for.
pub const MAX_TRACKED: usize = 10_000;
/// The characters of a normalised username the limiter keys on. Longer names cannot be
/// accounts, so a request cannot make one entry take more memory than this.
const MAX_USERNAME_KEY_CHARS: usize = 64;

/// Where the limiter reads the time from, so tests can move it.
pub(crate) trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

/// The limit on failed logins for every login route. Cloning it shares its counts.
#[derive(Clone)]
pub struct LoginLimiter {
    tables: Arc<Mutex<Tables>>,
    clock: Arc<dyn Clock>,
}

/// A login attempt the limiter let through. Report how it ended with [`Attempt::succeeded`] or
/// [`Attempt::failed`]; dropped without either, its count is given back.
#[must_use]
pub(crate) struct Attempt<'a> {
    limiter: &'a LoginLimiter,
    username: String,
    address: IpAddr,
    ended: bool,
}

/// The time of the real system.
struct SystemClock;

/// The counts of usernames and of addresses.
struct Tables {
    usernames: Table<String>,
    addresses: Table<IpAddr>,
}

/// The counts of one kind of key.
struct Table<K> {
    entries: HashMap<K, Entry>,
    limit: u32,
}

/// The failures counted for one key.
#[derive(Debug, Clone, Copy)]
struct Entry {
    failures: u32,
    window_start: Instant,
    locked_until: Option<Instant>,
}

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

impl Default for LoginLimiter {
    fn default() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }
}

impl LoginLimiter {
    /// A limiter with no counts that reads the time from `clock`.
    pub(crate) fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self {
            tables: Arc::new(Mutex::new(Tables {
                usernames: Table::new(USERNAME_FAILURE_LIMIT),
                addresses: Table::new(ADDRESS_FAILURE_LIMIT),
            })),
            clock,
        }
    }

    /// Start a login for `username` from `address`, counting it. `None` when either is locked,
    /// or has as many attempts counted as its limit: the login is refused without its password
    /// being checked.
    pub(crate) fn begin(&self, username: &str, address: IpAddr) -> Option<Attempt<'_>> {
        let username = username_key(username);
        let address = address.to_canonical();
        let now = self.clock.now();
        let mut tables = self.lock_tables();
        if !tables.usernames.admits(&username, now) || !tables.addresses.admits(&address, now) {
            return None;
        }
        tables.usernames.count(username.clone(), now);
        tables.addresses.count(address, now);
        Some(Attempt {
            limiter: self,
            username,
            address,
            ended: false,
        })
    }

    /// How many usernames and how many addresses have counts kept.
    #[cfg(test)]
    fn tracked(&self) -> (usize, usize) {
        let tables = self.lock_tables();
        (
            tables.usernames.entries.len(),
            tables.addresses.entries.len(),
        )
    }

    fn lock_tables(&self) -> std::sync::MutexGuard<'_, Tables> {
        self.tables.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Attempt<'_> {
    /// The password was right: clear the username's count and give back the address's.
    pub(crate) fn succeeded(mut self) {
        self.ended = true;
        let mut tables = self.limiter.lock_tables();
        tables.usernames.clear(&self.username);
        tables.addresses.give_back(&self.address);
    }

    /// The username is unknown or the password was wrong: keep the counts, and lock the
    /// username or the address that has reached its limit.
    pub(crate) fn failed(mut self) {
        self.ended = true;
        let now = self.limiter.clock.now();
        let (username_locked, address_locked) = {
            let mut tables = self.limiter.lock_tables();
            (
                tables.usernames.lock_if_full(&self.username, now),
                tables.addresses.lock_if_full(&self.address, now),
            )
        };
        if username_locked {
            tracing::warn!(
                username = %self.username,
                address = %self.address,
                failures = USERNAME_FAILURE_LIMIT,
                lockout_seconds = LOCKOUT.as_secs(),
                "Login locked for this username: too many failed logins"
            );
        }
        if address_locked {
            tracing::warn!(
                username = %self.username,
                address = %self.address,
                failures = ADDRESS_FAILURE_LIMIT,
                lockout_seconds = LOCKOUT.as_secs(),
                "Login locked for this address: too many failed logins"
            );
        }
    }
}

impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        if self.ended {
            return;
        }
        let mut tables = self.limiter.lock_tables();
        tables.usernames.give_back(&self.username);
        tables.addresses.give_back(&self.address);
    }
}

impl<K: Eq + Hash + Clone> Table<K> {
    fn new(limit: u32) -> Self {
        Self {
            entries: HashMap::new(),
            limit,
        }
    }

    /// Whether `key` may make another attempt now. An expired entry is dropped first.
    fn admits(&mut self, key: &K, now: Instant) -> bool {
        if self
            .entries
            .get(key)
            .is_some_and(|entry| entry.expired(now))
        {
            self.entries.remove(key);
        }
        self.entries
            .get(key)
            .is_none_or(|entry| entry.locked_until.is_none() && entry.failures < self.limit)
    }

    /// Count an attempt for `key`, making room for it when the table is full.
    fn count(&mut self, key: K, now: Instant) {
        if !self.entries.contains_key(&key) && self.entries.len() >= MAX_TRACKED {
            self.make_room(now);
        }
        self.entries
            .entry(key)
            .or_insert(Entry {
                failures: 0,
                window_start: now,
                locked_until: None,
            })
            .failures += 1;
    }

    /// Lock `key` when its count has reached the limit. `true` when this locked it.
    fn lock_if_full(&mut self, key: &K, now: Instant) -> bool {
        let Some(entry) = self.entries.get_mut(key) else {
            return false;
        };
        if entry.locked_until.is_some() || entry.failures < self.limit {
            return false;
        }
        entry.locked_until = Some(now + LOCKOUT);
        true
    }

    /// Take back one attempt counted for `key`, dropping the entry when none is left.
    fn give_back(&mut self, key: &K) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.failures = entry.failures.saturating_sub(1);
            if entry.failures == 0 && entry.locked_until.is_none() {
                self.entries.remove(key);
            }
        }
    }

    fn clear(&mut self, key: &K) {
        self.entries.remove(key);
    }

    /// Drop every expired entry, and when that frees nothing, the entry that matters least: the
    /// oldest one not locked, or else the lock that ends soonest.
    fn make_room(&mut self, now: Instant) {
        self.entries.retain(|_, entry| !entry.expired(now));
        if self.entries.len() < MAX_TRACKED {
            return;
        }
        let least = self
            .entries
            .iter()
            .min_by_key(|(_, entry)| match entry.locked_until {
                None => (false, entry.window_start),
                Some(until) => (true, until),
            })
            .map(|(key, _)| key.clone());
        if let Some(key) = least {
            self.entries.remove(&key);
        }
    }
}

impl Entry {
    /// Whether this entry no longer limits anything: its lock is over, or, not locked, its
    /// window has passed.
    fn expired(&self, now: Instant) -> bool {
        match self.locked_until {
            Some(until) => now >= until,
            None => now.duration_since(self.window_start) >= FAILURE_WINDOW,
        }
    }
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
    use std::net::Ipv4Addr;

    use super::*;
    use crate::log_capture::capture_log;

    const ADDRESS: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10));
    const OTHER_ADDRESS: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 20));

    /// A clock that moves only when a test moves it.
    struct ManualClock(Mutex<Instant>);

    impl Clock for ManualClock {
        fn now(&self) -> Instant {
            *self.0.lock().unwrap()
        }
    }

    impl ManualClock {
        fn advance(&self, by: Duration) {
            *self.0.lock().unwrap() += by;
        }
    }

    fn limiter() -> (LoginLimiter, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock(Mutex::new(Instant::now())));
        (LoginLimiter::with_clock(clock.clone()), clock)
    }

    fn fail(limiter: &LoginLimiter, username: &str, address: IpAddr, times: usize) {
        for _ in 0..times {
            limiter
                .begin(username, address)
                .expect("attempt let through")
                .failed();
        }
    }

    /// The address of the `n`th distinct client in a test, 10.x.y.z.
    fn nth_address(n: usize) -> IpAddr {
        let n = u32::try_from(n).expect("small n");
        IpAddr::V4(Ipv4Addr::from(0x0a00_0000 | n))
    }

    #[test]
    fn user_login_019_a_locked_username_is_let_in_again_15_minutes_after_it_was_locked() {
        let (limiter, clock) = limiter();
        fail(&limiter, "alice", ADDRESS, 5);
        assert!(limiter.begin("alice", OTHER_ADDRESS).is_none());

        clock.advance(Duration::from_secs(15 * 60 - 1));
        assert!(limiter.begin("alice", OTHER_ADDRESS).is_none());

        clock.advance(Duration::from_secs(1));
        assert!(limiter.begin("alice", OTHER_ADDRESS).is_some());
    }

    #[test]
    fn user_login_020_a_locked_address_is_let_in_again_15_minutes_after_it_was_locked() {
        let (limiter, clock) = limiter();
        for n in 0..20 {
            fail(&limiter, &format!("user{n}"), ADDRESS, 1);
        }
        assert!(limiter.begin("bob", ADDRESS).is_none());

        clock.advance(Duration::from_secs(15 * 60 - 1));
        assert!(limiter.begin("bob", ADDRESS).is_none());

        clock.advance(Duration::from_secs(1));
        assert!(limiter.begin("bob", ADDRESS).is_some());
    }

    #[test]
    fn user_login_021_failures_15_minutes_after_the_first_start_a_new_count() {
        let (limiter, clock) = limiter();
        fail(&limiter, "alice", ADDRESS, 4);
        clock.advance(Duration::from_mins(15));
        fail(&limiter, "alice", ADDRESS, 4);
        assert!(limiter.begin("alice", ADDRESS).is_some());
    }

    #[test]
    fn user_login_022_a_successful_login_clears_its_usernames_count() {
        let (limiter, _clock) = limiter();
        fail(&limiter, "alice", ADDRESS, 4);
        limiter
            .begin("alice", ADDRESS)
            .expect("attempt let through")
            .succeeded();
        fail(&limiter, "alice", ADDRESS, 4);
        assert!(limiter.begin("alice", ADDRESS).is_some());
    }

    #[test]
    fn user_login_023_a_successful_login_does_not_count_against_its_address() {
        let (limiter, _clock) = limiter();
        for n in 0..19 {
            fail(&limiter, &format!("user{n}"), ADDRESS, 1);
        }
        for _ in 0..5 {
            limiter
                .begin("alice", ADDRESS)
                .expect("attempt let through")
                .succeeded();
        }
        assert!(limiter.begin("bob", ADDRESS).is_some());
    }

    #[test]
    fn user_login_024_a_username_is_counted_in_any_case_and_with_surrounding_space() {
        let (limiter, _clock) = limiter();
        for username in ["alice", "Alice", " ALICE", "alice ", "aLiCe"] {
            fail(&limiter, username, ADDRESS, 1);
        }
        assert!(limiter.begin("alice", OTHER_ADDRESS).is_none());
    }

    #[test]
    fn user_login_025_an_ipv4_address_counts_alike_as_ipv4_and_as_ipv4_mapped_ipv6() {
        let (limiter, _clock) = limiter();
        let mapped = IpAddr::V6(Ipv4Addr::new(192, 0, 2, 10).to_ipv6_mapped());
        for n in 0..20 {
            fail(&limiter, &format!("user{n}"), mapped, 1);
        }
        assert!(limiter.begin("bob", ADDRESS).is_none());
    }

    #[test]
    fn user_login_026_attempts_made_at_once_check_no_more_passwords_than_the_limit() {
        let (limiter, _clock) = limiter();
        let in_flight: Vec<_> = (0..5)
            .map(|_| {
                limiter
                    .begin("alice", ADDRESS)
                    .expect("attempt let through")
            })
            .collect();
        assert!(limiter.begin("alice", OTHER_ADDRESS).is_none());
        drop(in_flight);
    }

    #[test]
    fn user_login_027_an_attempt_that_ends_without_an_answer_is_not_counted() {
        let (limiter, _clock) = limiter();
        for _ in 0..10 {
            drop(
                limiter
                    .begin("alice", ADDRESS)
                    .expect("attempt let through"),
            );
        }
        fail(&limiter, "alice", ADDRESS, 4);
        assert!(limiter.begin("alice", ADDRESS).is_some());
    }

    #[test]
    fn user_login_028_counts_are_kept_for_no_more_than_10000_usernames_and_addresses() {
        let (limiter, _clock) = limiter();
        for n in 0..10_100 {
            fail(&limiter, &format!("user{n}"), nth_address(n), 1);
        }
        assert_eq!(limiter.tracked(), (10_000, 10_000));
    }

    #[test]
    fn user_login_029_when_the_counts_are_full_a_lock_is_kept_over_a_count_that_is_not_one() {
        let (limiter, _clock) = limiter();
        fail(&limiter, "alice", ADDRESS, 5);
        for n in 0..10_100 {
            fail(&limiter, &format!("user{n}"), nth_address(n), 1);
        }
        assert!(limiter.begin("alice", OTHER_ADDRESS).is_none());
    }

    #[test]
    fn user_login_030_a_lockout_is_logged_once_at_warn_with_the_username_and_address() {
        let (limiter, _clock) = limiter();
        let ((), log) = capture_log(|| {
            fail(&limiter, "alice", ADDRESS, 5);
            for _ in 0..3 {
                assert!(limiter.begin("alice", ADDRESS).is_none());
            }
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
}
