use std::net::Ipv4Addr;

use askama::Template;

use template::ArubaAoscxTemplate;

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

/// The ciphertext of a random password nobody keeps, which locks `admin` and the owner.
const LOCKED: &str = "AQBapWi7gAb+1N611aRfybWmroBDwZuBCFmatCf0xTBVaKfvYgAAAG28Q0mssiKLsEYc6oFjpoOK7se339jlbWQZ6TgkNwA+Fds+jXCbsS/YVarj2xHh4ddtZwgVPlwNJnOn0qS3wdj67Tiu4rBBELar7IBH8ADbnYa4fUYPGL79+0g2OlJGNQ5p";

const EXPECTED_STATIC_IPV4: &str = "\
!
hostname aoscx01
domain-name lab.sherpa.local
ip dns server-address 172.20.0.1
user admin group administrators password ciphertext LOCKED
user alice group administrators password ciphertext LOCKED
user alice authorized-key ssh-rsa OWNERKEY1
user alice authorized-key ssh-ed25519 OWNERKEY2
!
ntp server pool.ntp.org minpoll 4 maxpoll 4 iburst
ntp enable
!
no ssh password-authentication
ssh server vrf mgmt
ssh sftp-server
vlan 1
interface mgmt
    no shutdown
    ip static 172.20.0.10/24
!
ip route 0.0.0.0/0 172.20.0.1
!
https-server vrf mgmt
!
lldp enable
!";

// ============================================================================
// Tests
// ============================================================================

fn aoscx() -> ArubaAoscxTemplate {
    let owner = helpers::test_owner("Rand0mPass-1");
    ArubaAoscxTemplate {
        hostname: "aoscx01".to_string(),
        username: owner.username,
        ssh_keys: owner.ssh_keys,
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
}

#[test]
fn test_static_ipv4() {
    let output = aoscx().render().expect("template renders");
    assert_eq!(output, EXPECTED_STATIC_IPV4.replace("LOCKED", LOCKED));
}

#[test]
fn test_only_the_owner_and_admin_have_accounts_and_both_are_locked() {
    let output = aoscx().render().expect("template renders");
    let users: Vec<&str> = output
        .lines()
        .filter(|line| line.starts_with("user "))
        .collect();
    assert_eq!(
        users,
        [
            format!("user admin group administrators password ciphertext {LOCKED}").as_str(),
            format!("user alice group administrators password ciphertext {LOCKED}").as_str(),
            "user alice authorized-key ssh-rsa OWNERKEY1",
            "user alice authorized-key ssh-ed25519 OWNERKEY2",
        ]
    );
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn test_no_password_in_plain_text() {
    let output = aoscx().render().expect("template renders");
    assert!(!output.contains("plaintext"));
    assert!(!output.contains("Rand0mPass-1"));
}

#[test]
fn test_ssh_password_login_is_off() {
    let output = aoscx().render().expect("template renders");
    assert!(output.contains("\nno ssh password-authentication\nssh server vrf mgmt\n"));
}
