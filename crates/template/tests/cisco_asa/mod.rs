// Note: This module is intentionally named cisco_ase to avoid collision with
// the cisco_asa directory. It tests the Cisco ASA template.

use std::net::Ipv4Addr;

use askama::Template;

use template::CiscoAsavZtpTemplate;

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_STATIC_IPV4: &str = "!
console serial
!
interface Management0/0
 nameif management
 management-only
 security-level 0
 ip address 172.20.0.10 255.255.255.0
 no shutdown
!
hostname asa01
!
username enable_1 privilege 15
enable password Everest1953!
username sherpa privilege 15 
username sherpa password Everest1953!
!
aaa authentication ssh console LOCAL
aaa authentication http console LOCAL
aaa authorization exec LOCAL auto-enable
no ssh stack ciscossh
crypto key generate rsa modulus 2048 noconfirm
ssh 0.0.0.0 0.0.0.0 management
ssh scopy enable
http server enable
http 0.0.0.0 0.0.0.0 management
domain-name lab.sherpa.local
route management 0.0.0.0 0.0.0.0 172.20.0.1
!
username sherpa attributes
  ssh authentication publickey AAAAB3NzaC1yc2EAAAADAQABAAABAQ hashed
  service-type admin
!
names
dns domain-lookup management
dns name-server 172.20.0.1
!";

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_static_ipv4() {
    let t = CiscoAsavZtpTemplate {
        hostname: "asa01".to_string(),
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
