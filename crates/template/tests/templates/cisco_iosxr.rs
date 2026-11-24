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
domain name sherpa.lab.local
domain name-server 172.20.0.2
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
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
}

#[test]
fn generated_config_012_cisco_iosxr() {
    let output = xrv("Rand0mPass-1").render().expect("template renders");
    assert_eq!(
        output,
        EXPECTED_STATIC_IPV4.replace("PASSWORD", "Rand0mPass-1")
    );
}

#[test]
fn device_accounts_006_cisco_iosxr_admin_and_owner() {
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

#[test]
fn config_name_server_010_cisco_iosxr() {
    let output = xrv("Rand0mPass-1").render().expect("template renders");
    assert_eq!(
        helpers::name_servers(&output, "domain name-server "),
        ["172.20.0.2"]
    );
}
