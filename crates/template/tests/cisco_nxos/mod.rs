use std::net::Ipv4Addr;

use askama::Template;

use template::CiscoNxosZtpTemplate;

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_STATIC_IPV4: &str = "\
!
feature bash-shell
feature nxapi
feature scp-server
feature lldp
!
hostname nxos01
!
username admin password 0 Everest1953! role network-admin
username sherpa password 0 Everest1953!
username sherpa role network-admin
username sherpa sshkey ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ
!
ip domain-name lab.sherpa.local
ip name-server 172.20.0.1
!
vrf context management
!
ip route 0.0.0.0/0 172.20.0.1 vrf management
!
interface mgmt0
  vrf member management
  ip address 172.20.0.10/24
  no shutdown
!
line vty
  exec-timeout 0
!";

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_static_ipv4() {
    let t = CiscoNxosZtpTemplate {
        hostname: "nxos01".to_string(),
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
