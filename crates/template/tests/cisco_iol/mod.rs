use std::net::Ipv4Addr;

use askama::Template;

use template::CiscoIolZtpTemplate;

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const PASSWORD: &str = "Rand0mPass-1";

const EXPECTED_ROUTER: &str = r"!
hostname r1
ip domain name lab.sherpa.local
ip name-server 172.20.0.1
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
crypto key generate rsa modulus 2048
ip ssh version 2
ip scp server enable
!
interface Ethernet0/0
 description management
 ip address 172.20.0.10 255.255.255.0
 no shutdown
!
ip route 0.0.0.0 0.0.0.0 172.20.0.1
!
line con 0
 logging synchronous
!
line vty 0 4
 logging synchronous
 transport input ssh
!
end";

// ============================================================================
// Tests
// ============================================================================

fn router(password: &str) -> CiscoIolZtpTemplate {
    CiscoIolZtpTemplate {
        hostname: "r1".to_string(),
        owner: helpers::test_owner(password),
        mgmt_interface: "Ethernet0/0".to_string(),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
        switch: false,
    }
}

#[test]
fn a_router_gets_its_accounts_ssh_and_a_static_management_address() {
    let output = router(PASSWORD).render().expect("template renders");
    assert_eq!(output, EXPECTED_ROUTER.replace("PASSWORD", PASSWORD));
}

#[test]
fn a_switch_makes_its_management_port_routed() {
    let output = CiscoIolZtpTemplate {
        hostname: "sw1".to_string(),
        switch: true,
        ..router(PASSWORD)
    }
    .render()
    .expect("template renders");
    assert!(output.contains(
        "interface Ethernet0/0\n \
         description management\n \
         no switchport\n \
         ip address 172.20.0.10 255.255.255.0\n \
         no shutdown\n!\n"
    ));
    assert!(output.contains("hostname sw1\n"));
    assert!(output.contains("ip routing\n"));
    assert!(output.contains("ip route 0.0.0.0 0.0.0.0 172.20.0.1\n"));
}

#[test]
fn a_router_has_no_switchport_line() {
    let output = router(PASSWORD).render().expect("template renders");
    assert!(!output.contains("switchport"));
    assert!(!output.contains("ip routing"));
}

#[test]
fn only_the_owner_has_an_account() {
    let output = router(PASSWORD).render().expect("template renders");
    let users: Vec<&str> = output
        .lines()
        .filter(|line| line.trim_start().starts_with("username "))
        .collect();
    assert_eq!(
        users,
        [
            "username alice privilege 15 secret Rand0mPass-1",
            "  username alice"
        ]
    );
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn the_password_is_intact() {
    for password in helpers::ALL_CHARACTER_PASSWORDS {
        let output = router(password).render().expect("template renders");
        assert!(output.contains(&format!(
            "\nusername alice privilege 15 secret {password}\n"
        )));
        assert_eq!(output.matches(password).count(), 1);
    }
}

#[test]
fn dual_stack_adds_an_ipv6_management_address_and_route() {
    let output = CiscoIolZtpTemplate {
        mgmt_ipv6_address: Some("fd00::10".parse().expect("valid")),
        mgmt_ipv6: Some(helpers::test_network_v6()),
        ..router(PASSWORD)
    }
    .render()
    .expect("template renders");
    assert!(output.contains(" ip address 172.20.0.10 255.255.255.0\n ipv6 address fd00::10/64\n"));
    assert!(output.contains("ipv6 unicast-routing\nipv6 route ::/0 fd00::1\n"));
}

#[test]
fn the_config_is_never_dhcp() {
    let output = router(PASSWORD).render().expect("template renders");
    assert!(!output.contains("dhcp"));
}
