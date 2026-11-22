//! One login key and the statements on its table, so the steps that count, lock and give back
//! run the same way on the pair, the address and the username tables.

use jiff::Timestamp;

use super::{LoginAddress, LoginExpiry, LoginKeys, LoginSubject};
use crate::queries;
use crate::sql::Bound;

/// A key the login limit counts attempts under, and the table that holds it.
#[derive(Debug, Clone, Copy)]
pub(super) enum Key<'a> {
    /// A username and an address together, in `login_limit_pair`.
    Pair {
        username: &'a str,
        address: LoginAddress,
    },
    /// An address, for every username, in `login_limit_address`.
    Address(LoginAddress),
    /// A username, from every address, in `login_limit_username`.
    Username(&'a str),
}

impl<'a> Key<'a> {
    /// The pair, address and username keys of `subject`.
    pub(super) fn of(subject: &'a LoginSubject) -> LoginKeys<Self> {
        LoginKeys {
            pair: Self::Pair {
                username: &subject.username,
                address: subject.address,
            },
            address: Self::Address(subject.address),
            username: Self::Username(&subject.username),
        }
    }

    /// The rows of this key's table, out of `sizes`.
    pub(super) fn rows(self, sizes: LoginKeys<usize>) -> usize {
        match self {
            Self::Pair { .. } => sizes.pair,
            Self::Address(_) => sizes.address,
            Self::Username(_) => sizes.username,
        }
    }

    /// Read the key's `failures` and `locked_until`.
    pub(super) fn find(self) -> Bound {
        match self {
            Self::Pair { username, address } => {
                queries::login_limit_pair::FIND.bind([username.into(), address.value()])
            }
            Self::Address(address) => queries::login_limit_address::FIND.bind([address.value()]),
            Self::Username(username) => queries::login_limit_username::FIND.bind([username.into()]),
        }
    }

    /// Delete the key's row when `expiry` says it limits nothing any more.
    pub(super) fn delete_expired_by_key(self, expiry: LoginExpiry) -> Bound {
        let [now, window_started_by] = expiry.values();
        match self {
            Self::Pair { username, address } => queries::login_limit_pair::DELETE_EXPIRED_BY_KEY
                .bind([username.into(), address.value(), now, window_started_by]),
            Self::Address(address) => queries::login_limit_address::DELETE_EXPIRED_BY_KEY.bind([
                address.value(),
                now,
                window_started_by,
            ]),
            Self::Username(username) => queries::login_limit_username::DELETE_EXPIRED_BY_KEY
                .bind([username.into(), now, window_started_by]),
        }
    }

    /// Delete every row of the key's table that `expiry` says limits nothing any more.
    pub(super) fn delete_expired(self, expiry: LoginExpiry) -> Bound {
        let values = expiry.values();
        match self {
            Self::Pair { .. } => queries::login_limit_pair::DELETE_EXPIRED.bind(values),
            Self::Address(_) => queries::login_limit_address::DELETE_EXPIRED.bind(values),
            Self::Username(_) => queries::login_limit_username::DELETE_EXPIRED.bind(values),
        }
    }

    /// Delete the oldest row of the key's table that is not locked.
    pub(super) fn delete_oldest_unlocked(self) -> Bound {
        match self {
            Self::Pair { .. } => queries::login_limit_pair::DELETE_OLDEST_UNLOCKED.bind([]),
            Self::Address(_) => queries::login_limit_address::DELETE_OLDEST_UNLOCKED.bind([]),
            Self::Username(_) => queries::login_limit_username::DELETE_OLDEST_UNLOCKED.bind([]),
        }
    }

    /// Count one more attempt on the key's row.
    pub(super) fn add_failure(self) -> Bound {
        match self {
            Self::Pair { username, address } => {
                queries::login_limit_pair::ADD_FAILURE.bind([username.into(), address.value()])
            }
            Self::Address(address) => {
                queries::login_limit_address::ADD_FAILURE.bind([address.value()])
            }
            Self::Username(username) => {
                queries::login_limit_username::ADD_FAILURE.bind([username.into()])
            }
        }
    }

    /// Add the key's row, with one attempt counted at `now`.
    pub(super) fn create(self, now: Timestamp) -> Bound {
        let now = now.as_second().into();
        match self {
            Self::Pair { username, address } => {
                queries::login_limit_pair::CREATE.bind([username.into(), address.value(), now])
            }
            Self::Address(address) => {
                queries::login_limit_address::CREATE.bind([address.value(), now])
            }
            Self::Username(username) => {
                queries::login_limit_username::CREATE.bind([username.into(), now])
            }
        }
    }

    /// Lock the key until `locked_until` when it is not locked and has `limit` attempts counted.
    /// Returns a row when this locked it.
    pub(super) fn lock_if_full(self, limit: u32, locked_until: Timestamp) -> Bound {
        let limit = i64::from(limit).into();
        let locked_until = locked_until.as_second().into();
        match self {
            Self::Pair { username, address } => queries::login_limit_pair::LOCK_IF_FULL.bind([
                username.into(),
                address.value(),
                limit,
                locked_until,
            ]),
            Self::Address(address) => queries::login_limit_address::LOCK_IF_FULL.bind([
                address.value(),
                limit,
                locked_until,
            ]),
            Self::Username(username) => queries::login_limit_username::LOCK_IF_FULL.bind([
                username.into(),
                limit,
                locked_until,
            ]),
        }
    }

    /// Delete the key's row.
    pub(super) fn delete_by_key(self) -> Bound {
        match self {
            Self::Pair { username, address } => {
                queries::login_limit_pair::DELETE_BY_KEY.bind([username.into(), address.value()])
            }
            Self::Address(address) => {
                queries::login_limit_address::DELETE_BY_KEY.bind([address.value()])
            }
            Self::Username(username) => {
                queries::login_limit_username::DELETE_BY_KEY.bind([username.into()])
            }
        }
    }

    /// Take one counted attempt back from the key's row.
    pub(super) fn take_back_failure(self) -> Bound {
        match self {
            Self::Pair { username, address } => queries::login_limit_pair::TAKE_BACK_FAILURE
                .bind([username.into(), address.value()]),
            Self::Address(address) => {
                queries::login_limit_address::TAKE_BACK_FAILURE.bind([address.value()])
            }
            Self::Username(username) => {
                queries::login_limit_username::TAKE_BACK_FAILURE.bind([username.into()])
            }
        }
    }

    /// Delete the key's row when it has no attempts counted and is not locked.
    pub(super) fn delete_if_empty(self) -> Bound {
        match self {
            Self::Pair { username, address } => {
                queries::login_limit_pair::DELETE_IF_EMPTY.bind([username.into(), address.value()])
            }
            Self::Address(address) => {
                queries::login_limit_address::DELETE_IF_EMPTY.bind([address.value()])
            }
            Self::Username(username) => {
                queries::login_limit_username::DELETE_IF_EMPTY.bind([username.into()])
            }
        }
    }
}
