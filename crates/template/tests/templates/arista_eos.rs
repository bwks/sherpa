use std::net::Ipv4Addr;

use askama::Template;

use template::{AristaEosContainerZtpTemplate, AristaEosVmZtpTemplate};

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

// vEOS uses Management1
const EXPECTED_VEOS_STATIC_IPV4: &str = "\
!
hostname veos01
dns domain sherpa.lab.local
ip name-server 172.20.0.2
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
dns domain sherpa.lab.local
ip name-server 172.20.0.2
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
dns domain sherpa.lab.local
ip name-server 172.20.0.2
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

fn veos(hash: &str, ipv6: bool) -> AristaEosVmZtpTemplate {
    AristaEosVmZtpTemplate {
        hostname: "veos01".to_string(),
        owner: helpers::test_owner(hash),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: ipv6.then(|| "fd00::10".parse().expect("valid")),
        mgmt_ipv6: ipv6.then(helpers::test_network_v6),
    }
}

fn ceos(hash: &str) -> AristaEosContainerZtpTemplate {
    AristaEosContainerZtpTemplate {
        hostname: "ceos01".to_string(),
        owner: helpers::test_owner(hash),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
}

#[test]
fn generated_config_001_arista_veos() {
    let hash = helpers::sha512_hash(PASSWORD);
    let output = veos(&hash, false).render().expect("template renders");
    assert_eq!(output, EXPECTED_VEOS_STATIC_IPV4.replace("HASH", &hash));
}

#[test]
fn generated_config_002_arista_ceos() {
    let hash = helpers::sha512_hash(PASSWORD);
    let output = ceos(&hash).render().expect("template renders");
    assert_eq!(output, EXPECTED_CEOS_STATIC_IPV4.replace("HASH", &hash));
}

#[test]
fn config_addressing_007_arista_veos_dual_stack() {
    let hash = helpers::sha512_hash(PASSWORD);
    let output = veos(&hash, true).render().expect("template renders");
    assert_eq!(output, EXPECTED_VEOS_DUAL_STACK.replace("HASH", &hash));
}

#[test]
fn config_name_server_002_arista_veos() {
    let output = veos(&helpers::sha512_hash(PASSWORD), false)
        .render()
        .expect("template renders");
    assert_eq!(
        helpers::name_servers(&output, "ip name-server "),
        ["172.20.0.2"]
    );
}

#[test]
fn config_name_server_003_arista_ceos() {
    let output = ceos(&helpers::sha512_hash(PASSWORD))
        .render()
        .expect("template renders");
    assert_eq!(
        helpers::name_servers(&output, "ip name-server "),
        ["172.20.0.2"]
    );
}
