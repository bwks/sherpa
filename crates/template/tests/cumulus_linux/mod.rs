use std::net::Ipv4Addr;

use askama::Template;

use template::CumulusLinuxZtpTemplate;

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_STATIC_IPV4: &str = r#"#!/bin/bash

# CUMULUS-AUTOPROVISIONING

function error() {
  echo -e "\e[0;33mERROR: The ZTP script failed while running the command $BASH_COMMAND at line $BASH_LINENO.\e[0m" >&2
  exit 1
}

# Log all output from this script
exec >> /var/log/autoprovision 2>&1
date "+%FT%T ztp starting script $0"

trap error ERR

#Configs
nv set system hostname cumulus01
nv set service dns default search lab.sherpa.local

nv set service dns default server 172.20.0.1

nv set system aaa user alice
nv set system aaa user alice password 'PASSWORD'
nv set system aaa user alice ssh authorized-key alice-ssh-key-1 key OWNERKEY1
nv set system aaa user alice ssh authorized-key alice-ssh-key-1 type ssh-rsa
nv set system aaa user alice ssh authorized-key alice-ssh-key-2 key OWNERKEY2
nv set system aaa user alice ssh authorized-key alice-ssh-key-2 type ssh-ed25519
nv set system aaa user alice role system-admin
nv unset interface eth0 ip address dhcp
nv set interface eth0 ip address 172.20.0.10/24
nv set interface eth0 ip gateway 172.20.0.1

nv config apply --assume-yes --message "ZTP config"

exit 0"#;

// ============================================================================
// Tests
// ============================================================================

fn cumulus(password: &str) -> CumulusLinuxZtpTemplate {
    CumulusLinuxZtpTemplate {
        hostname: "cumulus01".to_string(),
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
    let output = cumulus("Rand0mPass-1").render().expect("template renders");
    assert_eq!(
        output,
        EXPECTED_STATIC_IPV4.replace("PASSWORD", "Rand0mPass-1")
    );
}

#[test]
fn test_password_is_intact() {
    for password in helpers::ALL_CHARACTER_PASSWORDS {
        let output = cumulus(password).render().expect("template renders");
        assert!(output.contains(&format!(
            "\nnv set system aaa user alice password '{password}'\n"
        )));
        assert_eq!(output.matches(password).count(), 1);
    }
}
