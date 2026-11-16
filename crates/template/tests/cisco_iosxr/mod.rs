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
 secret 0 PASSWORD
!
username alice
 group root-lr
 group netadmin
 secret 0 PASSWORD
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
ssh server username alice
 keystring ssh-rsa OWNERKEY1
 keystring ssh-ed25519 second OWNERKEY2
!
ssh server v2
ssh server vrf default
ssh server netconf vrf default";

// ============================================================================
// Tests
// ============================================================================

fn xrv(password: &str) -> CiscoIosxrZtpTemplate {
    CiscoIosxrZtpTemplate {
        hostname: "xrv01".to_string(),
        owner: helpers::test_owner(password),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
}

#[test]
fn test_static_ipv4() {
    let output = xrv("Rand0mPass-1").render().expect("template renders");
    assert_eq!(
        output,
        EXPECTED_STATIC_IPV4.replace("PASSWORD", "Rand0mPass-1")
    );
}

#[test]
fn test_only_admin_and_the_owner_have_accounts() {
    let output = xrv("Rand0mPass-1").render().expect("template renders");
    let users: Vec<&str> = output
        .lines()
        .filter(|line| line.starts_with("username ") || line.starts_with("ssh server username"))
        .collect();
    assert_eq!(
        users,
        [
            "username admin",
            "username alice",
            "ssh server username alice"
        ]
    );
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn test_password_is_intact_for_admin_and_owner() {
    for password in helpers::ALL_CHARACTER_PASSWORDS {
        let output = xrv(password).render().expect("template renders");
        assert_eq!(
            output.matches(&format!("\n secret 0 {password}\n")).count(),
            2
        );
        assert_eq!(output.matches(password).count(), 2);
        assert!(output.contains(&format!(
            "username admin\n group root-lr\n group cisco-support\n secret 0 {password}\n"
        )));
        assert!(output.contains(&format!(
            "username alice\n group root-lr\n group netadmin\n secret 0 {password}\n"
        )));
    }
}
