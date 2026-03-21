use std::net::Ipv4Addr;

use askama::Template;

use template::CiscoIosXeZtpTemplate;

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_STATIC_IPV4: &str = "\
!
hostname csr01
ip domain name lab.sherpa.local
ip name-server 172.20.0.1
crypto key generate rsa modulus 2048
ip ssh version 2
!
aaa new-model
aaa authentication login default local
aaa authorization exec default local
!
username sherpa privilege 15 secret Everest1953!
!
ip ssh pubkey-chain
  username sherpa
   key-hash ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ
!
ip scp server enable
!

!
lldp run
!
archive
 log config
  logging enable
  logging size 1000
  notify syslog contenttype plaintext
 path flash:/archived-config
 maximum 14
 write-memory
 time-period 1440
!
interface GigabitEthernet1
 ip address 172.20.0.10 255.255.255.0
 negotiation auto
 no shutdown
 exit
!
ip route 0.0.0.0 0.0.0.0 172.20.0.1
!
line con 0
 logging synchronous
 stopbits 1
 exit
!
line vty 0 4
 logging synchronous
 transport input ssh
 exit
!
exit";

const EXPECTED_WITH_LICENSE: &str = "\
!
hostname csr01
ip domain name lab.sherpa.local
ip name-server 172.20.0.1
crypto key generate rsa modulus 2048
ip ssh version 2
!
aaa new-model
aaa authentication login default local
aaa authorization exec default local
!
username sherpa privilege 15 secret Everest1953!
!
ip ssh pubkey-chain
  username sherpa
   key-hash ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ
!
ip scp server enable
!
license boot level network-advantage
!
lldp run
!
archive
 log config
  logging enable
  logging size 1000
  notify syslog contenttype plaintext
 path flash:/archived-config
 maximum 14
 write-memory
 time-period 1440
!
interface GigabitEthernet1
 ip address 172.20.0.10 255.255.255.0
 negotiation auto
 no shutdown
 exit
!
ip route 0.0.0.0 0.0.0.0 172.20.0.1
!
line con 0
 logging synchronous
 stopbits 1
 exit
!
line vty 0 4
 logging synchronous
 transport input ssh
 exit
!
exit";

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_static_ipv4() {
    let t = CiscoIosXeZtpTemplate {
        hostname: "csr01".to_string(),
        user: helpers::test_user(),
        mgmt_interface: "GigabitEthernet1".to_string(),
        dns: helpers::test_dns(),
        license_boot_command: None,
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_STATIC_IPV4);
}

#[test]
fn test_with_license_command() {
    let t = CiscoIosXeZtpTemplate {
        hostname: "csr01".to_string(),
        user: helpers::test_user(),
        mgmt_interface: "GigabitEthernet1".to_string(),
        dns: helpers::test_dns(),
        license_boot_command: Some("license boot level network-advantage".to_string()),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_WITH_LICENSE);
}
