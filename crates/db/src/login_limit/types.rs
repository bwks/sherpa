//! The keys, limits, expiry and counts the login limit's rows are made of.

use jiff::Timestamp;
use turso::Value;

/// Which count a row of the login limit is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginKeyKind {
    /// A username and an address together.
    Pair,
    /// An address, for every username.
    Address,
    /// A username, from every address.
    Username,
}

/// A key the login limit counts attempts under. `username` is empty for an address key and
/// `address` for a username key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginKey {
    kind: LoginKeyKind,
    username: String,
    address: String,
}

/// A key with the number of counted attempts that locks it.
#[derive(Debug, Clone, Copy)]
pub struct LimitedKey<'a> {
    pub key: &'a LoginKey,
    pub limit: u32,
}

/// The moment a step runs at, and so which rows no longer limit anything: those whose lock has
/// ended by `now`, and those not locked whose window started at or before `window_started_by`.
#[derive(Debug, Clone, Copy)]
pub struct LoginExpiry {
    pub now: Timestamp,
    pub window_started_by: Timestamp,
}

/// The counted attempts of one row and the end of its lock.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LoginCount {
    pub(crate) failures: u32,
    pub(crate) locked_until: Option<Timestamp>,
}

impl LoginKeyKind {
    /// The kind as the `kind` column stores it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pair => "pair",
            Self::Address => "address",
            Self::Username => "username",
        }
    }
}

impl LoginKey {
    /// The key of `username` and `address` together.
    #[must_use]
    pub fn pair(username: &str, address: &str) -> Self {
        Self {
            kind: LoginKeyKind::Pair,
            username: username.to_owned(),
            address: address.to_owned(),
        }
    }

    /// The key of `address`, for every username.
    #[must_use]
    pub fn address(address: &str) -> Self {
        Self {
            kind: LoginKeyKind::Address,
            username: String::new(),
            address: address.to_owned(),
        }
    }

    /// The key of `username`, from every address.
    #[must_use]
    pub fn username(username: &str) -> Self {
        Self {
            kind: LoginKeyKind::Username,
            username: username.to_owned(),
            address: String::new(),
        }
    }

    /// Which count this key is.
    #[must_use]
    pub fn kind(&self) -> LoginKeyKind {
        self.kind
    }

    /// The key's `kind`, `username` and `address` column values.
    pub(super) fn values(&self) -> [Value; 3] {
        [
            self.kind.as_str().into(),
            self.username.as_str().into(),
            self.address.as_str().into(),
        ]
    }
}

impl LoginExpiry {
    /// The `now` and `window_started_by` column values, as Unix seconds.
    pub(super) fn values(self) -> [Value; 2] {
        [
            self.now.as_second().into(),
            self.window_started_by.as_second().into(),
        ]
    }
}
