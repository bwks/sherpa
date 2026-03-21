use std::net::Ipv4Addr;

use askama::Template;

use template::CiscoIosvZtpTemplate;

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_STATIC_IPV4: &str = "\
!
hostname iosv01
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
interface GigabitEthernet0/0
 ip address 172.20.0.10 255.255.255.0
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
event manager applet ENABLE-MGMT
 event syslog pattern \"SYS-5-RESTART\"
 action 0 cli command \"enable\"
 action 1 cli command \"conf t\"
 action 2 cli command \"interface GigabitEthernet0/0\"
 action 3 cli command \"no shutdown\"
 action 4 cli command \"exit\"
 action 5 cli command \"crypto key generate rsa modulus 2048\"
!
exit";

const EXPECTED_DUAL_STACK: &str = "\
!
hostname iosv01
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
interface GigabitEthernet0/0
 ip address 172.20.0.10 255.255.255.0
 ipv6 address fd00::10/64
 no shutdown
 exit
!
ip route 0.0.0.0 0.0.0.0 172.20.0.1
ipv6 unicast-routing
ipv6 route ::/0 fd00::1
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
event manager applet ENABLE-MGMT
 event syslog pattern \"SYS-5-RESTART\"
 action 0 cli command \"enable\"
 action 1 cli command \"conf t\"
 action 2 cli command \"interface GigabitEthernet0/0\"
 action 3 cli command \"no shutdown\"
 action 4 cli command \"exit\"
 action 5 cli command \"crypto key generate rsa modulus 2048\"
!
exit";

const EXPECTED_DHCP: &str = "\
!
hostname iosv01
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
interface GigabitEthernet0/0
 ip address dhcp
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
event manager applet ENABLE-MGMT
 event syslog pattern \"SYS-5-RESTART\"
 action 0 cli command \"enable\"
 action 1 cli command \"conf t\"
 action 2 cli command \"interface GigabitEthernet0/0\"
 action 3 cli command \"no shutdown\"
 action 4 cli command \"exit\"
 action 5 cli command \"crypto key generate rsa modulus 2048\"
!
exit";

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_static_ipv4() {
    let t = CiscoIosvZtpTemplate {
        hostname: "iosv01".to_string(),
        user: helpers::test_user(),
        mgmt_interface: "GigabitEthernet0/0".to_string(),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_STATIC_IPV4);
}

#[test]
fn test_dual_stack() {
    let t = CiscoIosvZtpTemplate {
        hostname: "iosv01".to_string(),
        user: helpers::test_user(),
        mgmt_interface: "GigabitEthernet0/0".to_string(),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: Some("fd00::10".parse().expect("valid")),
        mgmt_ipv6: Some(helpers::test_network_v6()),
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_DUAL_STACK);
}

#[test]
fn test_dhcp_mode() {
    let t = CiscoIosvZtpTemplate {
        hostname: "iosv01".to_string(),
        user: helpers::test_user(),
        mgmt_interface: "GigabitEthernet0/0".to_string(),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: None,
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_DHCP);
}
