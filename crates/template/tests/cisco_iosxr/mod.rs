use std::net::Ipv4Addr;

use askama::Template;

use template::CiscoIosxrZtpTemplate;

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_STATIC_IPV4: &str = "\
!
hostname xrv01
domain name lab.sherpa.local
domain name-server 172.20.0.1
username admin
 group root-lr
 group cisco-support
 secret 0 Everest1953!
!
username sherpa
 group root-lr
 group netadmin
 secret 0 Everest1953!
!
aaa authorization exec default local
aaa authentication login default local
!
lldp
!
netconf-yang agent
 ssh
!
interface MgmtEth0/RP0/CPU0/0
 ipv4 address 172.20.0.10/24
 no shutdown
!
router static
 address-family ipv4 unicast
  0.0.0.0/0 172.20.0.1
!
!
netconf agent tty
 session timeout 5
!
ssh server logging
!
ssh server username sherpa
 keystring ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ
!
ssh server v2
ssh server vrf default
ssh server netconf vrf default";

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_static_ipv4() {
    let t = CiscoIosxrZtpTemplate {
        hostname: "xrv01".to_string(),
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
