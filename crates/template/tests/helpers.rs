use std::net::Ipv4Addr;

use ipnet::{Ipv4Net, Ipv6Net};

use sha_crypt::{PasswordVerifier, ShaCrypt};

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

/// Logins and account lines the shared `sherpa` account used to write, one per device
/// format. Not the bare word: `sherpa` is also in the domain, heredoc markers and paths.
const SHERPA_LOGINS: &[&str] = &[
    "username sherpa",
    "user sherpa",
    "name sherpa",
    "\"sherpa\"",
    "useradd sherpa",
    "adduser sherpa",
    "adduser --disabled-password --gecos \"\" sherpa",
    "sudoers.d/sherpa",
    "sherpa ALL=",
    "name=\"sherpa\"",
    "entry name=\"sherpa\"",
    "username=sherpa",
    "name: sherpa",
    "name = sherpa",
    "/home/sherpa",
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

/// `password` as a `$5$` SHA-256 hash.
///
/// # Panics
///
/// If the hash cannot be computed.
#[must_use]
pub fn sha256_hash(password: &str) -> String {
    crypt_hash(password, CryptHash::Sha256).expect("hash computes")
}

/// The hash that follows the first `marker` in `output`: up to the next space, quote, angle
/// bracket, semicolon or end of line.
///
/// # Panics
///
/// If `marker` is not in `output`.
#[must_use]
pub fn hash_after(output: &str, marker: &str) -> String {
    let start = output.find(marker).expect("marker is in the output") + marker.len();
    output[start..]
        .chars()
        .take_while(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | '<' | ';'))
        .collect()
}

/// Every hash that follows a `marker` in `output`, in order.
#[must_use]
pub fn hashes_after(output: &str, marker: &str) -> Vec<String> {
    output
        .match_indices(marker)
        .map(|(index, _)| hash_after(&output[index..], marker))
        .collect()
}

/// Asserts that `hash` is a crypt hash of `password` and starts with `prefix`.
///
/// # Panics
///
/// If the prefix is wrong or the password does not verify.
pub fn assert_hash_verifies(hash: &str, prefix: &str, password: &str) {
    assert!(
        hash.starts_with(prefix),
        "{hash} does not start with {prefix}"
    );
    assert!(
        ShaCrypt::default()
            .verify_password(password.as_bytes(), hash)
            .is_ok(),
        "{hash} is not a hash of the password"
    );
}

/// Asserts that no `sherpa` login is in `output`.
///
/// # Panics
///
/// If one is found.
pub fn assert_no_sherpa_login(output: &str) {
    for login in SHERPA_LOGINS {
        assert!(
            !output.contains(login),
            "a sherpa login ({login}) is in the output:\n{output}"
        );
    }
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
