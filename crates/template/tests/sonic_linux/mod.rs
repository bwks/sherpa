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
fn test_sonic_user_template() {
    let hash = helpers::sha512_hash("abcdefghijklm!@_-+.");
    let output = user_script(&hash).render().expect("template renders");
    assert_eq!(output, EXPECTED_SONIC_USER.replace("HASH", &hash));
}

#[test]
fn test_sonic_user_template_has_only_the_owner_account() {
    let hash = helpers::sha512_hash("abcdefghijklm!@_-+.");
    let output = user_script(&hash).render().expect("template renders");
    assert_eq!(output.matches("useradd").count(), 1);
    assert_eq!(output.matches("chpasswd").count(), 1);
    assert_eq!(output.matches("/etc/sudoers.d/").count(), 2);
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn test_sonic_user_password_is_a_hash_that_verifies() {
    for password in helpers::ALL_CHARACTER_PASSWORDS {
        let hash = helpers::sha512_hash(password);
        let output = user_script(&hash).render().expect("template renders");
        let found = helpers::hash_after(&output, "alice:");
        assert_eq!(found, hash);
        helpers::assert_hash_verifies(&found, "$6$", password);
        assert!(output.contains("| /usr/sbin/chpasswd -e\n"));
        assert!(!output.contains(password));
    }
}

#[test]
fn test_sonic_file_map() {
    let server = Ipv4Addr::new(172, 20, 0, 1);
    let output = SonicLinuxZtp::file_map("sonic01", &server);
    assert_eq!(output, EXPECTED_FILE_MAP);
}

#[test]
fn test_sonic_file_map_merges_and_saves_the_config() {
    let server = Ipv4Addr::new(172, 20, 0, 1);
    let map = parsed(&SonicLinuxZtp::file_map("sonic01", &server));
    let step = &map["ztp"]["001-configdb-json"];
    assert_eq!(step["clear-config"], json!(false));
    assert_eq!(step["save-config"], json!(true));
}

#[test]
fn test_sonic_file_map_loads_the_config_again_without_saving_it() {
    let server = Ipv4Addr::new(172, 20, 0, 1);
    let map = parsed(&SonicLinuxZtp::file_map("sonic01", &server));
    let first = &map["ztp"]["001-configdb-json"];
    let again = &map["ztp"]["003-configdb-json"];
    assert_eq!(again["url"]["source"], first["url"]["source"]);
    // No destination, so the plugin deletes the download and the saved config stays whole.
    assert!(again["url"].get("destination").is_none());
    assert_eq!(again["clear-config"], json!(false));
    assert_eq!(again["save-config"], json!(false));
    let steps: Vec<&String> = map["ztp"].as_object().expect("steps").keys().collect();
    assert_eq!(
        steps,
        ["001-configdb-json", "002-set-password", "003-configdb-json"]
    );
}

#[test]
fn test_sonic_file_map_still_runs_the_owner_account_script() {
    let server = Ipv4Addr::new(172, 20, 0, 1);
    let map = parsed(&SonicLinuxZtp::file_map("sonic01", &server));
    let step = &map["ztp"]["002-set-password"];
    assert_eq!(
        step["plugin"]["url"],
        json!(format!(
            "http://172.20.0.1:8080/{}",
            SonicLinuxZtp::user_script_filename("sonic01")
        ))
    );
    assert_eq!(step["plugin"]["shell"], json!("true"));
    assert_eq!(step["reboot-on-success"], json!(false));
}

#[test]
fn test_sonic_config_has_32_ports_all_admin_up() {
    for address in [None, Some(Ipv4Addr::new(172, 20, 0, 10))] {
        let config = parsed(&sonic_ztp(address).config());
        let ports = config["PORT"].as_object().expect("a PORT table");
        assert_eq!(ports.len(), 32);
        for name in EXPECTED_PORTS {
            assert_eq!(ports[name], json!({ "admin_status": "up" }), "{name}");
        }
    }
}

#[test]
fn test_sonic_config_static_ip() {
    let mut expected = expected_base_config();
    expected["MGMT_INTERFACE"] = json!({ "eth0|172.20.0.10/24": { "gwaddr": "172.20.0.1" } });
    expected["MGMT_PORT"] = json!({ "eth0": { "admin_status": "up", "alias": "eth0" } });
    let output = sonic_ztp(Some(Ipv4Addr::new(172, 20, 0, 10))).config();
    assert_eq!(parsed(&output), expected);
}

#[test]
fn test_sonic_config_dhcp() {
    let output = sonic_ztp(None).config();
    assert_eq!(parsed(&output), expected_base_config());
}
