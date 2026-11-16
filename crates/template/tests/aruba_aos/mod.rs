use std::net::Ipv4Addr;

use askama::Template;

use template::ArubaAoscxTemplate;

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_STATIC_IPV4: &str = "\
!
hostname aoscx01
domain-name lab.sherpa.local
ip dns server-address 172.20.0.1
user admin group administrators password plaintext PASSWORD
user alice group administrators password plaintext PASSWORD
user alice authorized-key ssh-rsa OWNERKEY1
user alice authorized-key ssh-ed25519 OWNERKEY2
!
ntp server pool.ntp.org minpoll 4 maxpoll 4 iburst
ntp enable
!
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

fn aoscx(password: &str) -> ArubaAoscxTemplate {
    ArubaAoscxTemplate {
        hostname: "aoscx01".to_string(),
        owner: helpers::test_owner(password),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
}

#[test]
fn test_static_ipv4() {
    let output = aoscx("Rand0mPass-1").render().expect("template renders");
    assert_eq!(
        output,
        EXPECTED_STATIC_IPV4.replace("PASSWORD", "Rand0mPass-1")
    );
}

#[test]
fn test_only_the_owner_and_admin_have_accounts() {
    let output = aoscx("Rand0mPass-1").render().expect("template renders");
    let users: Vec<&str> = output
        .lines()
        .filter(|line| line.starts_with("user "))
        .collect();
    assert_eq!(
        users,
        [
            "user admin group administrators password plaintext Rand0mPass-1",
            "user alice group administrators password plaintext Rand0mPass-1",
            "user alice authorized-key ssh-rsa OWNERKEY1",
            "user alice authorized-key ssh-ed25519 OWNERKEY2",
        ]
    );
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn test_password_is_intact_for_admin_and_owner() {
    for password in helpers::ALL_CHARACTER_PASSWORDS {
        let output = aoscx(password).render().expect("template renders");
        assert!(output.contains(&format!(
            "user admin group administrators password plaintext {password}\n"
        )));
        assert!(output.contains(&format!(
            "user alice group administrators password plaintext {password}\n"
        )));
        assert_eq!(output.matches(password).count(), 2);
    }
}
