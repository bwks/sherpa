use std::net::Ipv4Addr;

use askama::Template;
use serde_json::{Map, Value, json};

use template::{SonicLinuxUserTemplate, SonicLinuxZtp};

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_SONIC_USER: &str = r"#!/bin/bash

# The owner logs in by key, or with their device password, set from its SHA-512-crypt hash.
/usr/sbin/useradd -m -s /bin/bash -G sudo,admin,docker alice
/usr/bin/echo 'alice:HASH' | /usr/sbin/chpasswd -e
/usr/bin/mkdir -p /home/alice/.ssh
/usr/bin/cat >/home/alice/.ssh/authorized_keys <<'SHERPA_KEYS'
ssh-rsa OWNERKEY1
ssh-ed25519 OWNERKEY2
SHERPA_KEYS
/usr/bin/chmod 700 /home/alice/.ssh
/usr/bin/chmod 600 /home/alice/.ssh/authorized_keys
/usr/bin/chown -R alice:alice /home/alice
/usr/bin/echo 'alice ALL=(ALL) NOPASSWD: ALL' >/etc/sudoers.d/alice
/usr/bin/chmod 440 /etc/sudoers.d/alice

exit 0";

// file_map() uses json!().to_string() which produces compact JSON.
// HTTP_PORT=8080
const EXPECTED_FILE_MAP: &str = r#"{"ztp":{"001-configdb-json":{"clear-config":false,"save-config":true,"url":{"destination":"/etc/sonic/config_db.json","secure":false,"source":"http://172.20.0.1:8080/sonic01_config_db.json"}},"002-set-password":{"plugin":{"shell":"true","url":"http://172.20.0.1:8080/sonic01_ztp_user.sh"},"reboot-on-success":false},"003-configdb-json":{"clear-config":false,"save-config":false,"url":{"secure":false,"source":"http://172.20.0.1:8080/sonic01_config_db.json"}}}}"#;

/// The front-panel ports of the `Force10-S6000` SKU the image uses.
const EXPECTED_PORTS: [&str; 32] = [
    "Ethernet0",
    "Ethernet4",
    "Ethernet8",
    "Ethernet12",
    "Ethernet16",
    "Ethernet20",
    "Ethernet24",
    "Ethernet28",
    "Ethernet32",
    "Ethernet36",
    "Ethernet40",
    "Ethernet44",
    "Ethernet48",
    "Ethernet52",
    "Ethernet56",
    "Ethernet60",
    "Ethernet64",
    "Ethernet68",
    "Ethernet72",
    "Ethernet76",
    "Ethernet80",
    "Ethernet84",
    "Ethernet88",
    "Ethernet92",
    "Ethernet96",
    "Ethernet100",
    "Ethernet104",
    "Ethernet108",
    "Ethernet112",
    "Ethernet116",
    "Ethernet120",
    "Ethernet124",
];

/// The config every node gets: hostname, local login and every port admin up.
fn expected_base_config() -> Value {
    let ports: Map<String, Value> = EXPECTED_PORTS
        .iter()
        .map(|port| ((*port).to_string(), json!({ "admin_status": "up" })))
        .collect();
    json!({
        "AAA": { "authentication": { "login": "local" } },
        "DEVICE_METADATA": { "localhost": { "hostname": "sonic01" } },
        "PORT": ports,
    })
}

fn sonic_ztp(mgmt_ipv4_address: Option<Ipv4Addr>) -> SonicLinuxZtp {
    SonicLinuxZtp {
        hostname: "sonic01".to_string(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address,
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
}

fn parsed(output: &str) -> Value {
    serde_json::from_str(output).expect("valid JSON")
}

// ============================================================================
// Tests
// ============================================================================

fn user_script(hash: &str) -> SonicLinuxUserTemplate {
    SonicLinuxUserTemplate {
        owner: helpers::test_owner(hash),
    }
}

#[test]
fn device_accounts_019_sonic_account_script_makes_the_owner_with_keys_hash_and_sudo() {
    let hash = helpers::sha512_hash("abcdefghijklm!@_-+.");
    let output = user_script(&hash).render().expect("template renders");
    assert_eq!(output, EXPECTED_SONIC_USER.replace("HASH", &hash));
}

#[test]
fn config_delivery_017_http_ztp_file_names_its_config_and_account_script_by_url() {
    let server = Ipv4Addr::new(172, 20, 0, 1);
    let output = SonicLinuxZtp::file_map("sonic01", &server);
    assert_eq!(output, EXPECTED_FILE_MAP);
}

#[test]
fn generated_config_024_sonic_config_db() {
    let mut expected = expected_base_config();
    expected["MGMT_INTERFACE"] = json!({ "eth0|172.20.0.10/24": { "gwaddr": "172.20.0.1" } });
    expected["MGMT_PORT"] = json!({ "eth0": { "admin_status": "up", "alias": "eth0" } });
    let output = sonic_ztp(Some(Ipv4Addr::new(172, 20, 0, 10))).config();
    assert_eq!(output, expected.to_string());
}

#[test]
fn config_addressing_012_a_sonic_container_has_no_management_address() {
    let output = sonic_ztp(None).config();
    assert_eq!(parsed(&output), expected_base_config());
}
