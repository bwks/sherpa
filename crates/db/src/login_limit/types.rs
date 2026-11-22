//! The keys, counts, window and failures the login limit's rows are made of.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use jiff::Timestamp;
use turso::Value;

/// The prefix length an IPv6 address is counted under.
const IPV6_PREFIX_LEN: u32 = 64;
/// The bytes of an IPv6 address that are its counted prefix.
const IPV6_PREFIX_BYTES: usize = 8;

/// A source address as the login limit counts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginAddress {
    /// An IPv4 address, counted alone.
    V4(Ipv4Addr),
    /// The first 8 bytes, the /64 prefix, of an IPv6 address.
    V6Prefix([u8; IPV6_PREFIX_BYTES]),
}

/// One login's username and address, which make its pair, address and username keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginSubject {
    /// The username as the limiter counts it.
    pub username: String,
    pub address: LoginAddress,
}

/// One value for each key of a login: its username and address pair, its address and its
/// username.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginKeys<T> {
    pub pair: T,
    pub address: T,
    pub username: T,
}

/// The moment an attempt starts at, and the rows it counts: those failed after `expired_by`.
#[derive(Debug, Clone, Copy)]
pub struct LoginWindow {
    pub now: Timestamp,
    /// The latest moment a failure no longer counts at: `now` less the window.
    pub expired_by: Timestamp,
}

/// One failure row an attempt wrote when it started, so the attempt can delete it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginFailureId {
    rowid: i64,
    failed_at: Timestamp,
}

/// Whether the login limit lets an attempt through, and what it counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginAdmission {
    /// A key has as many failures in the window as its limit; no row is written.
    Refused,
    /// Every key is under its limit.
    Admitted {
        /// The failures in the window for the pair, the address and the username, the attempt's
        /// own row included when it was written.
        counts: LoginKeys<u32>,
        /// The attempt's own row, `None` when the table was at its cap.
        failure: Option<LoginFailureId>,
    },
}

impl LoginAddress {
    /// The key `address` is counted under: an IPv4 address itself, an IPv6 address its /64
    /// prefix. An IPv4-mapped IPv6 address is counted as its IPv4 address.
    #[must_use]
    pub fn of(address: IpAddr) -> Self {
        match address.to_canonical() {
            IpAddr::V4(v4) => Self::V4(v4),
            IpAddr::V6(v6) => {
                let mut prefix = [0; IPV6_PREFIX_BYTES];
                prefix.copy_from_slice(&v6.octets()[..IPV6_PREFIX_BYTES]);
                Self::V6Prefix(prefix)
            }
        }
    }

    /// The `address` column value: the 4 bytes of an IPv4 address or the 8 of an IPv6 prefix.
    pub(super) fn value(self) -> Value {
        match self {
            Self::V4(v4) => v4.octets().to_vec().into(),
            Self::V6Prefix(prefix) => prefix.to_vec().into(),
        }
    }
}

impl fmt::Display for LoginAddress {
    /// An IPv4 address as itself and an IPv6 prefix as `2001:db8:1:2::/64`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::V4(v4) => v4.fmt(f),
            Self::V6Prefix(prefix) => {
                let mut octets = [0; 16];
                octets[..IPV6_PREFIX_BYTES].copy_from_slice(prefix);
                write!(f, "{}/{IPV6_PREFIX_LEN}", Ipv6Addr::from(octets))
            }
        }
    }
}

impl LoginFailureId {
    /// The row `rowid`, written at `failed_at`.
    pub(super) fn new(rowid: i64, failed_at: Timestamp) -> Self {
        Self { rowid, failed_at }
    }

    /// The `rowid` and `failed_at` column values.
    pub(super) fn values(self) -> [Value; 2] {
        [self.rowid.into(), self.failed_at.as_second().into()]
    }
}
