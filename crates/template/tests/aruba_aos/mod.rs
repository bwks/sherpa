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
user admin group administrators password plaintext Everest1953!
user sherpa  group administrators password plaintext Everest1953!
user sherpa authorized-key ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ
!
ntp server pool.ntp.org minpoll 4 maxpoll 4 iburst
ntp enable
!
ssh server vrf mgmt
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

#[test]
fn test_static_ipv4() {
    let t = ArubaAoscxTemplate {
        hostname: "aoscx01".to_string(),
        user: helpers::test_user(),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_STATIC_IPV4);
}
