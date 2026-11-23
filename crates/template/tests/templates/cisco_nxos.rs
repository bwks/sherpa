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
username admin password 0 PASSWORD role network-admin
username alice password 0 PASSWORD role network-admin
username alice sshkey ssh-rsa OWNERKEY1
username alice sshkey ssh-ed25519 OWNERKEY2
!
ip domain-name sherpa.lab.local
ip name-server 172.20.0.2
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

fn nxos(password: &str) -> CiscoNxosZtpTemplate {
    CiscoNxosZtpTemplate {
        hostname: "nxos01".to_string(),
        owner: helpers::test_owner(password),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
}

#[test]
fn generated_config_014_cisco_nxos() {
    let output = nxos("Rand0mPass-1").render().expect("template renders");
    assert_eq!(
        output,
        EXPECTED_STATIC_IPV4.replace("PASSWORD", "Rand0mPass-1")
    );
}

#[test]
fn device_accounts_008_cisco_nxos_admin_and_owner() {
    for password in helpers::ALL_CHARACTER_PASSWORDS {
        let output = nxos(password).render().expect("template renders");
        assert!(output.contains(&format!(
            "\nusername admin password 0 {password} role network-admin\n"
        )));
        assert!(output.contains(&format!(
            "\nusername alice password 0 {password} role network-admin\n"
        )));
        assert_eq!(output.matches(password).count(), 2);
    }
}

#[test]
fn config_name_server_011_cisco_nxos() {
    let output = nxos("Rand0mPass-1").render().expect("template renders");
    assert_eq!(
        helpers::name_servers(&output, "ip name-server "),
        ["172.20.0.2"]
    );
}
