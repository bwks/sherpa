use std::net::Ipv4Addr;

use askama::Template;

use template::{CiscoIosZtpTemplate, CiscoIosl2ZtpTemplate};

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const PASSWORD: &str = "Rand0mPass-1";

const EXPECTED_STATIC_IPV4: &str = r#"!
hostname iosv01
ip domain name sherpa.lab.local
ip name-server 172.20.0.2
crypto key generate rsa modulus 2048
ip ssh version 2
!
aaa new-model
aaa authentication login default local
aaa authorization exec default local
!
username alice privilege 15 secret PASSWORD
!
ip ssh pubkey-chain
  username alice
   key-hash ssh-rsa OWNERKEY1
   key-hash ssh-ed25519 OWNERKEY2
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
 transport preferred none
 stopbits 1
 exit
!
line vty 0 4
 logging synchronous
 transport preferred none
 transport input ssh
 exit
!
event manager applet ENABLE-MGMT
 event syslog pattern "SYS-5-RESTART"
 action 0 cli command "enable"
 action 1 cli command "conf t"
 action 2 cli command "interface GigabitEthernet0/0"
 action 3 cli command "no shutdown"
 action 4 cli command "exit"
 action 5 cli command "crypto key generate rsa modulus 2048"
!
do write memory
!
exit"#;

const EXPECTED_DUAL_STACK: &str = r#"!
hostname iosv01
ip domain name sherpa.lab.local
ip name-server 172.20.0.2
crypto key generate rsa modulus 2048
ip ssh version 2
!
aaa new-model
aaa authentication login default local
aaa authorization exec default local
!
username alice privilege 15 secret PASSWORD
!
ip ssh pubkey-chain
  username alice
   key-hash ssh-rsa OWNERKEY1
   key-hash ssh-ed25519 OWNERKEY2
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
 transport preferred none
 stopbits 1
 exit
!
line vty 0 4
 logging synchronous
 transport preferred none
 transport input ssh
 exit
!
event manager applet ENABLE-MGMT
 event syslog pattern "SYS-5-RESTART"
 action 0 cli command "enable"
 action 1 cli command "conf t"
 action 2 cli command "interface GigabitEthernet0/0"
 action 3 cli command "no shutdown"
 action 4 cli command "exit"
 action 5 cli command "crypto key generate rsa modulus 2048"
!
do write memory
!
exit"#;

// ============================================================================
// Tests
// ============================================================================

#[test]
fn generated_config_008_cisco_iosv() {
    let t = CiscoIosZtpTemplate {
        hostname: "iosv01".to_string(),
        owner: helpers::test_owner(PASSWORD),
        mgmt_interface: "GigabitEthernet0/0".to_string(),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_STATIC_IPV4.replace("PASSWORD", PASSWORD));
}

#[test]
fn config_addressing_008_cisco_iosv_dual_stack() {
    let t = CiscoIosZtpTemplate {
        hostname: "iosv01".to_string(),
        owner: helpers::test_owner(PASSWORD),
        mgmt_interface: "GigabitEthernet0/0".to_string(),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: Some("fd00::10".parse().expect("valid")),
        mgmt_ipv6: Some(helpers::test_network_v6()),
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_DUAL_STACK.replace("PASSWORD", PASSWORD));
}

#[test]
fn device_accounts_004_cisco_iosv_and_iosvl2_owner_only() {
    for password in helpers::ALL_CHARACTER_PASSWORDS {
        let ios = CiscoIosZtpTemplate {
            hostname: "iosv01".to_string(),
            owner: helpers::test_owner(password),
            mgmt_interface: "GigabitEthernet0/0".to_string(),
            dns: helpers::test_dns(),
            mgmt_ipv4: helpers::test_network_v4(),
            mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
            mgmt_ipv6_address: None,
            mgmt_ipv6: None,
        };
        let iosl2 = CiscoIosl2ZtpTemplate {
            hostname: "iosl2".to_string(),
            owner: helpers::test_owner(password),
            mgmt_interface: "GigabitEthernet0/0".to_string(),
            dns: helpers::test_dns(),
            mgmt_ipv4: helpers::test_network_v4(),
            mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
            mgmt_ipv6_address: None,
            mgmt_ipv6: None,
        };
        for output in [
            ios.render().expect("template renders"),
            iosl2.render().expect("template renders"),
        ] {
            let users: Vec<&str> = output
                .lines()
                .filter(|line| line.trim_start().starts_with("username "))
                .collect();
            assert_eq!(
                users,
                [
                    format!("username alice privilege 15 secret {password}"),
                    "  username alice".to_string()
                ]
            );
            assert_eq!(output.matches(password).count(), 1);
        }
    }
}

#[test]
fn config_name_server_007_cisco_iosv() {
    let output = CiscoIosZtpTemplate {
        hostname: "iosv01".to_string(),
        owner: helpers::test_owner(PASSWORD),
        mgmt_interface: "GigabitEthernet0/0".to_string(),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
    .render()
    .expect("template renders");
    assert_eq!(
        helpers::name_servers(&output, "ip name-server "),
        ["172.20.0.2"]
    );
}

#[test]
fn config_name_server_008_cisco_iosvl2() {
    let output = CiscoIosl2ZtpTemplate {
        hostname: "iosl2".to_string(),
        owner: helpers::test_owner(PASSWORD),
        mgmt_interface: "GigabitEthernet0/0".to_string(),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
    .render()
    .expect("template renders");
    assert_eq!(
        helpers::name_servers(&output, "ip name-server "),
        ["172.20.0.2"]
    );
}
