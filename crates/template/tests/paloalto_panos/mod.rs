use std::net::Ipv4Addr;

use askama::Template;

use template::{PaloAltoPanosBootstrapTemplate, PaloAltoPanosZtpTemplate};

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_INIT_IPV4: &str = "\
type=static
ip-address=172.20.0.10
default-gateway=172.20.0.1
netmask=255.255.255.0
hostname=panos01
dns-primary=172.20.0.1
dns-secondary=8.8.8.8";

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_panos_init_static_ipv4() {
    let t = PaloAltoPanosZtpTemplate {
        hostname: "panos01".to_string(),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
        mgmt_netmask: Ipv4Addr::new(255, 255, 255, 0),
        mgmt_gateway: Ipv4Addr::new(172, 20, 0, 1),
        dns_primary: Ipv4Addr::new(172, 20, 0, 1),
        dns_secondary: Ipv4Addr::new(8, 8, 8, 8),
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_INIT_IPV4);
}

fn bootstrap(hash: &str) -> PaloAltoPanosBootstrapTemplate {
    PaloAltoPanosBootstrapTemplate {
        hostname: "panos01".to_string(),
        owner: helpers::test_owner(hash),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
        mgmt_netmask: Ipv4Addr::new(255, 255, 255, 0),
        mgmt_gateway: Ipv4Addr::new(172, 20, 0, 1),
        dns_primary: Ipv4Addr::new(172, 20, 0, 1),
    }
}

/// The text between `<users>` and `</users>`.
fn users_block(output: &str) -> &str {
    let start = output.find("<users>").expect("has users");
    let end = output.find("</users>").expect("has users end");
    &output[start..end]
}

#[test]
fn test_bootstrap_users_are_admin_and_the_owner() {
    let hash = helpers::sha256_hash("abcdefghijklm!@_-+.");
    let output = bootstrap(&hash).render().expect("template renders");
    let users = users_block(&output);
    let entries: Vec<&str> = users
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("<entry name="))
        .collect();
    assert_eq!(
        entries,
        ["<entry name=\"admin\">", "<entry name=\"alice\">"]
    );
    // Only the owner has keys.
    let owner_entry = &users[users.find("<entry name=\"alice\">").expect("owner")..];
    assert!(!users[..users.len() - owner_entry.len()].contains("<public-key>"));
    assert!(owner_entry.contains("<public-key>OWNERKEY1</public-key>"));
    assert!(owner_entry.contains("<public-key>OWNERKEY2</public-key>"));
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn test_bootstrap_password_is_a_hash_that_verifies_for_admin_and_owner() {
    for password in helpers::ALL_CHARACTER_PASSWORDS {
        let hash = helpers::sha256_hash(password);
        let output = bootstrap(&hash).render().expect("template renders");
        let found = helpers::hashes_after(&output, "<phash>");
        assert_eq!(found.len(), 2);
        for found in &found {
            assert_eq!(found, &hash);
            helpers::assert_hash_verifies(found, "$5$", password);
        }
        assert!(!output.contains(password));
    }
}
