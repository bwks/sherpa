use std::net::Ipv4Addr;

use ipnet::{Ipv4Net, Ipv6Net};

use shared::data::{Dns, NameServer, NetworkV4, NetworkV6, OwnerSshKey, OwnerUser};
use template::{CryptHash, crypt_hash};

/// Device passwords that together use every character a device password may have: `a-z`,
/// `A-Z`, `0-9` and `! @ _ - + .`. Each is 12 to 20 characters, starts with a letter or a
/// digit and has all six punctuation characters.
pub const ALL_CHARACTER_PASSWORDS: &[&str] = &[
    "abcdefghijklm!@_-+.",
    "nopqrstuvwxyz.+-_@!",
    "ABCDEFGHIJKLM!@_-+.",
    "NOPQRSTUVWXYZ+.-_@!",
    "0123456789!@_-+.Ab",
];

/// A lab owner with two keys, as the generators pass them to device templates, with
/// `password` as the device password.
#[must_use]
pub fn test_owner(password: &str) -> OwnerUser {
    OwnerUser {
        username: "alice".to_string(),
        password: password.to_string(),
        ssh_keys: vec![
            OwnerSshKey {
                kind: "ssh-rsa".to_string(),
                value: "OWNERKEY1".to_string(),
            },
            OwnerSshKey {
                kind: "ssh-ed25519".to_string(),
                value: "OWNERKEY2".to_string(),
            },
        ],
    }
}

/// `password` as a `$6$` SHA-512 hash.
///
/// # Panics
///
/// If the hash cannot be computed.
#[must_use]
pub fn sha512_hash(password: &str) -> String {
    crypt_hash(password, CryptHash::Sha512).expect("hash computes")
}

#[must_use]
pub fn test_dns() -> Dns {
    Dns {
        domain: "lab.sherpa.local".to_string(),
        name_servers: vec![NameServer {
            ipv4_address: Ipv4Addr::new(172, 20, 0, 1),
            ipv6_address: None,
        }],
    }
}

/// The management network `172.20.0.0/24`.
///
/// # Panics
///
/// Never: the prefix is a valid literal.
#[must_use]
pub fn test_network_v4() -> NetworkV4 {
    NetworkV4 {
        prefix: "172.20.0.0/24".parse::<Ipv4Net>().expect("valid prefix"),
        first: Ipv4Addr::new(172, 20, 0, 1),
        last: Ipv4Addr::new(172, 20, 0, 254),
        boot_server: Ipv4Addr::new(172, 20, 0, 1),
        network: Ipv4Addr::new(172, 20, 0, 0),
        subnet_mask: Ipv4Addr::new(255, 255, 255, 0),
        hostmask: Ipv4Addr::new(0, 0, 0, 255),
        prefix_length: 24,
    }
}

/// The management network `fd00::/64`.
///
/// # Panics
///
/// Never: the prefix and addresses are valid literals.
#[must_use]
pub fn test_network_v6() -> NetworkV6 {
    NetworkV6 {
        prefix: "fd00::/64".parse::<Ipv6Net>().expect("valid prefix"),
        first: "fd00::1".parse().expect("valid addr"),
        last: "fd00::ffff".parse().expect("valid addr"),
        boot_server: "fd00::1".parse().expect("valid addr"),
        network: "fd00::".parse().expect("valid addr"),
        prefix_length: 64,
    }
}
