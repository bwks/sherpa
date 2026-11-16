use std::net::Ipv4Addr;

use askama::Template;

use template::{AristaCeosZtpTemplate, AristaVeosZtpTemplate};

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

// vEOS uses Management1
const EXPECTED_VEOS_STATIC_IPV4: &str = "\
!
hostname veos01
dns domain lab.sherpa.local
ip name-server 172.20.0.1
!
no aaa root
!
service routing protocols model multi-agent
!
aaa authorization exec default local
!
username alice privilege 15 secret sha512 HASH
username alice ssh-key ssh-rsa OWNERKEY1
username alice ssh-key secondary ssh-ed25519 OWNERKEY2
!
ip route 0.0.0.0/0 172.20.0.1
!
interface Management1
   ip address 172.20.0.10/24
!
management api http-commands
   no shutdown
!
lldp run
!
end
!";

// cEOS uses Management0
const EXPECTED_CEOS_STATIC_IPV4: &str = "\
!
hostname ceos01
dns domain lab.sherpa.local
ip name-server 172.20.0.1
!
no aaa root
!
service routing protocols model multi-agent
!
aaa authorization exec default local
!
username alice privilege 15 secret sha512 HASH
username alice ssh-key ssh-rsa OWNERKEY1
username alice ssh-key secondary ssh-ed25519 OWNERKEY2
!
ip route 0.0.0.0/0 172.20.0.1
!
interface Management0
   ip address 172.20.0.10/24
!
management api http-commands
   no shutdown
!
lldp run
!
end
!";

const EXPECTED_VEOS_DUAL_STACK: &str = "\
!
hostname veos01
dns domain lab.sherpa.local
ip name-server 172.20.0.1
!
no aaa root
!
service routing protocols model multi-agent
!
aaa authorization exec default local
!
username alice privilege 15 secret sha512 HASH
username alice ssh-key ssh-rsa OWNERKEY1
username alice ssh-key secondary ssh-ed25519 OWNERKEY2
!
ip route 0.0.0.0/0 172.20.0.1
ipv6 route ::/0 fd00::1
!
interface Management1
   ip address 172.20.0.10/24
   ipv6 address fd00::10/64
!
management api http-commands
   no shutdown
!
lldp run
!
end
!";

// ============================================================================
// Tests
// ============================================================================

const PASSWORD: &str = "abcdefghijklm!@_-+.";

fn veos(hash: &str, ipv6: bool) -> AristaVeosZtpTemplate {
    AristaVeosZtpTemplate {
        hostname: "veos01".to_string(),
        owner: helpers::test_owner(hash),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: ipv6.then(|| "fd00::10".parse().expect("valid")),
        mgmt_ipv6: ipv6.then(helpers::test_network_v6),
    }
}

fn ceos(hash: &str) -> AristaCeosZtpTemplate {
    AristaCeosZtpTemplate {
        hostname: "ceos01".to_string(),
        owner: helpers::test_owner(hash),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
}

#[test]
fn test_veos_static_ipv4() {
    let hash = helpers::sha512_hash(PASSWORD);
    let output = veos(&hash, false).render().expect("template renders");
    assert_eq!(output, EXPECTED_VEOS_STATIC_IPV4.replace("HASH", &hash));
}

#[test]
fn test_ceos_static_ipv4() {
    let hash = helpers::sha512_hash(PASSWORD);
    let output = ceos(&hash).render().expect("template renders");
    assert_eq!(output, EXPECTED_CEOS_STATIC_IPV4.replace("HASH", &hash));
}

#[test]
fn test_veos_dual_stack() {
    let hash = helpers::sha512_hash(PASSWORD);
    let output = veos(&hash, true).render().expect("template renders");
    assert_eq!(output, EXPECTED_VEOS_DUAL_STACK.replace("HASH", &hash));
}

#[test]
fn test_only_the_owner_has_an_account() {
    let hash = helpers::sha512_hash(PASSWORD);
    for output in [
        veos(&hash, false).render().expect("template renders"),
        ceos(&hash).render().expect("template renders"),
    ] {
        let users: Vec<&str> = output
            .lines()
            .filter(|line| line.starts_with("username "))
            .collect();
        assert_eq!(users.len(), 3);
        assert!(users.iter().all(|line| line.starts_with("username alice ")));
        helpers::assert_no_sherpa_login(&output);
    }
}

#[test]
fn test_owner_password_is_a_hash_that_verifies() {
    for password in helpers::ALL_CHARACTER_PASSWORDS {
        let hash = helpers::sha512_hash(password);
        for output in [
            veos(&hash, false).render().expect("template renders"),
            ceos(&hash).render().expect("template renders"),
        ] {
            let found = helpers::hash_after(&output, "secret sha512 ");
            assert_eq!(found, hash);
            helpers::assert_hash_verifies(&found, "$6$", password);
            assert!(!output.contains(password));
        }
    }
}
