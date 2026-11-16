use std::net::Ipv4Addr;

use askama::Template;

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
// HTTP_PORT=8080, NODE_CONFIGS_DIR="configs"
const EXPECTED_FILE_MAP: &str = r#"{"ztp":{"001-configdb-json":{"url":{"destination":"/etc/sonic/config_db.json","secure":false,"source":"http://172.20.0.1:8080/configs/sonic01_config_db.json"}},"002-set-password":{"plugin":{"shell":"true","url":"http://172.20.0.1:8080/configs/sonic01_ztp_user.sh"},"reboot-on-success":false}}}"#;

// config() uses json!().to_string() which produces compact JSON.
// With static IP: includes MGMT_PORT and MGMT_INTERFACE keys.
const EXPECTED_CONFIG_STATIC: &str = r#"{"AAA":{"authentication":{"login":"local"}},"DEVICE_METADATA":{"localhost":{"hostname":"sonic01"}},"MGMT_INTERFACE":{"eth0|172.20.0.10/24":{"gwaddr":"172.20.0.1"}},"MGMT_PORT":{"eth0":{"admin_status":"up","alias":"eth0"}}}"#;

// Without static IP: no MGMT_PORT or MGMT_INTERFACE keys.
const EXPECTED_CONFIG_DHCP: &str = r#"{"AAA":{"authentication":{"login":"local"}},"DEVICE_METADATA":{"localhost":{"hostname":"sonic01"}}}"#;

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
