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
fn generated_config_010_cisco_iosxe_stops_pnp_first() {
    let t = CiscoIosXeZtpTemplate {
        hostname: "csr01".to_string(),
        owner: helpers::test_owner(PASSWORD),
        mgmt_interface: "GigabitEthernet1".to_string(),
        dns: helpers::test_dns(),
        license_boot_command: None,
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_STATIC_IPV4.replace("PASSWORD", PASSWORD));
}

#[test]
fn generated_config_011_cisco_iosxe_with_a_boot_licence() {
    let t = CiscoIosXeZtpTemplate {
        hostname: "csr01".to_string(),
        owner: helpers::test_owner(PASSWORD),
        mgmt_interface: "GigabitEthernet1".to_string(),
        dns: helpers::test_dns(),
        license_boot_command: Some("license boot level network-advantage".to_string()),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_WITH_LICENSE.replace("PASSWORD", PASSWORD));
}

#[test]
fn device_accounts_005_cisco_iosxe_owner_only() {
    for password in helpers::ALL_CHARACTER_PASSWORDS {
        let t = CiscoIosXeZtpTemplate {
            hostname: "csr01".to_string(),
            owner: helpers::test_owner(password),
            mgmt_interface: "GigabitEthernet1".to_string(),
            dns: helpers::test_dns(),
            license_boot_command: None,
            mgmt_ipv4: helpers::test_network_v4(),
            mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
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
fn config_name_server_009_cisco_iosxe() {
    let output = CiscoIosXeZtpTemplate {
        hostname: "csr01".to_string(),
        owner: helpers::test_owner(PASSWORD),
        mgmt_interface: "GigabitEthernet1".to_string(),
        dns: helpers::test_dns(),
        license_boot_command: None,
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
