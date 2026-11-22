//! The keys, limits, expiry and counts the login limit's rows are made of.

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

/// What ending an attempt does to one of its keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginRelease {
    /// Leave the key's count as it is.
    Keep,
    /// Take the attempt back from the key's count.
    GiveBack,
    /// Delete the key's count.
    Clear,
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

impl<T> LoginKeys<T> {
    /// Each key's value made into another by `f`.
    pub fn map<U>(self, mut f: impl FnMut(T) -> U) -> LoginKeys<U> {
        LoginKeys {
            pair: f(self.pair),
            address: f(self.address),
            username: f(self.username),
        }
    }
}

impl LoginRelease {
    /// [`Self::GiveBack`] for a key that counted the attempt, [`Self::Keep`] for one that did not.
    #[must_use]
    pub fn give_back_if(counted: bool) -> Self {
        if counted { Self::GiveBack } else { Self::Keep }
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
