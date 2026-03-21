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
username sherpa privilege 15 secret Everest1953!
username sherpa ssh-key ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ
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
username sherpa privilege 15 secret Everest1953!
username sherpa ssh-key ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ
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
username sherpa privilege 15 secret Everest1953!
username sherpa ssh-key ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ
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

#[test]
fn test_veos_static_ipv4() {
    let t = AristaVeosZtpTemplate {
        hostname: "veos01".to_string(),
        user: helpers::test_user(),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_VEOS_STATIC_IPV4);
}

#[test]
fn test_ceos_static_ipv4() {
    let t = AristaCeosZtpTemplate {
        hostname: "ceos01".to_string(),
        user: helpers::test_user(),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_CEOS_STATIC_IPV4);
}

#[test]
fn test_veos_dual_stack() {
    let t = AristaVeosZtpTemplate {
        hostname: "veos01".to_string(),
        user: helpers::test_user(),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: Some("fd00::10".parse().expect("valid")),
        mgmt_ipv6: Some(helpers::test_network_v6()),
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_VEOS_DUAL_STACK);
}
