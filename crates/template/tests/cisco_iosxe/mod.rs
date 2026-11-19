use std::net::Ipv4Addr;

use askama::Template;

use template::CiscoIosXeZtpTemplate;

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const PASSWORD: &str = "Rand0mPass-1";

const EXPECTED_STATIC_IPV4: &str = "\
!
do pnpa service discovery stop
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
username alice privilege 15 secret PASSWORD
!
ip ssh pubkey-chain
  username alice
   key-hash ssh-rsa OWNERKEY1
   key-hash ssh-ed25519 OWNERKEY2
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
exit";

const EXPECTED_WITH_LICENSE: &str = "\
!
do pnpa service discovery stop
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
username alice privilege 15 secret PASSWORD
!
ip ssh pubkey-chain
  username alice
   key-hash ssh-rsa OWNERKEY1
   key-hash ssh-ed25519 OWNERKEY2
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
exit";

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_static_ipv4() {
    let t = CiscoIosXeZtpTemplate {
        hostname: "csr01".to_string(),
        owner: helpers::test_owner(PASSWORD),
        mgmt_interface: "GigabitEthernet1".to_string(),
        dns: helpers::test_dns(),
        license_boot_command: None,
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_STATIC_IPV4.replace("PASSWORD", PASSWORD));
}

#[test]
fn test_with_license_command() {
    let t = CiscoIosXeZtpTemplate {
        hostname: "csr01".to_string(),
        owner: helpers::test_owner(PASSWORD),
        mgmt_interface: "GigabitEthernet1".to_string(),
        dns: helpers::test_dns(),
        license_boot_command: Some("license boot level network-advantage".to_string()),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_WITH_LICENSE.replace("PASSWORD", PASSWORD));
}

#[test]
fn only_the_owner_has_an_account_and_the_password_is_intact() {
    for password in helpers::ALL_CHARACTER_PASSWORDS {
        let t = CiscoIosXeZtpTemplate {
            hostname: "csr01".to_string(),
            owner: helpers::test_owner(password),
            mgmt_interface: "GigabitEthernet1".to_string(),
            dns: helpers::test_dns(),
            license_boot_command: None,
            mgmt_ipv4: helpers::test_network_v4(),
            mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
            mgmt_ipv6_address: None,
            mgmt_ipv6: None,
        };
        let output = t.render().expect("template renders");
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

#[test]
fn pnp_discovery_stops_before_the_management_interface_comes_up() {
    let t = CiscoIosXeZtpTemplate {
        hostname: "csr01".to_string(),
        owner: helpers::test_owner(PASSWORD),
        mgmt_interface: "GigabitEthernet1".to_string(),
        dns: helpers::test_dns(),
        license_boot_command: None,
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    };
    let output = t.render().expect("template renders");
    let lines: Vec<&str> = output.lines().collect();
    let stop = lines
        .iter()
        .position(|line| *line == "do pnpa service discovery stop")
        .expect("the config stops PnP discovery");
    let interface = lines
        .iter()
        .position(|line| *line == "interface GigabitEthernet1")
        .expect("the config sets up the management interface");
    let first_command = lines
        .iter()
        .position(|line| *line != "!")
        .expect("the config has a command");
    assert_eq!(stop, first_command, "PnP stops before anything else");
    assert!(stop < interface);
}
