use std::net::Ipv4Addr;

use askama::Template;

use template::{SonicLinuxUserTemplate, SonicLinuxZtp};

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_SONIC_USER: &str = "\
#!/bin/bash

/usr/sbin/useradd sherpa
/usr/bin/echo \"sherpa:Everest1953!\" | chpasswd
/usr/bin/mkdir -p /home/sherpa/.ssh
/usr/bin/echo 'ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ' >/home/sherpa/.ssh/authorized_keys
/usr/bin/chmod 700 /home/sherpa/.ssh
/usr/bin/chmod 600 /home/sherpa/.ssh/authorized_keys
/usr/bin/chown -R sherpa:sherpa /home/sherpa
/usr/sbin/usermod -s /bin/bash sherpa

/usr/sbin/usermod -aG sudo,admin,docker sherpa


exit 0";

// file_map() uses json!().to_string() which produces compact JSON.
// HTTP_PORT=8080, NODE_CONFIGS_DIR="configs"
const EXPECTED_FILE_MAP: &str = r#"{"ztp":{"001-configdb-json":{"url":{"destination":"/etc/sonic/config_db.json","secure":false,"source":"http://172.20.0.1:8080/configs/sonic01_config_db.json"}},"002-set-password":{"plugin":{"shell":"true","url":"http://172.20.0.1:8080/configs/sonic_ztp_user.sh"},"reboot-on-success":false}}}"#;

// config() uses json!().to_string() which produces compact JSON.
// With static IP: includes MGMT_PORT and MGMT_INTERFACE keys.
const EXPECTED_CONFIG_STATIC: &str = r#"{"AAA":{"authentication":{"login":"local"}},"DEVICE_METADATA":{"localhost":{"hostname":"sonic01"}},"MGMT_INTERFACE":{"eth0|172.20.0.10/24":{"gwaddr":"172.20.0.1"}},"MGMT_PORT":{"eth0":{"admin_status":"up","alias":"eth0"}}}"#;

// Without static IP: no MGMT_PORT or MGMT_INTERFACE keys.
const EXPECTED_CONFIG_DHCP: &str = r#"{"AAA":{"authentication":{"login":"local"}},"DEVICE_METADATA":{"localhost":{"hostname":"sonic01"}}}"#;

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_sonic_user_template() {
    let t = SonicLinuxUserTemplate {
        user: helpers::test_user(),
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_SONIC_USER);
}

#[test]
fn test_sonic_file_map() {
    let server = Ipv4Addr::new(172, 20, 0, 1);
    let output = SonicLinuxZtp::file_map("sonic01", &server);
    assert_eq!(output, EXPECTED_FILE_MAP);
}

#[test]
fn test_sonic_config_static_ip() {
    let ztp = SonicLinuxZtp {
        hostname: "sonic01".to_string(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    };
    let output = ztp.config();
    assert_eq!(output, EXPECTED_CONFIG_STATIC);
}

#[test]
fn test_sonic_config_dhcp() {
    let ztp = SonicLinuxZtp {
        hostname: "sonic01".to_string(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: None,
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    };
    let output = ztp.config();
    assert_eq!(output, EXPECTED_CONFIG_DHCP);
}
