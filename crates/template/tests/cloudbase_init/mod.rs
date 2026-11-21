use std::net::Ipv4Addr;

use serde_json::{Value, json};
use template::{CloudbaseInitConfig, CloudbaseInitNetwork, CloudbaseInitUser};

use crate::helpers;

// Guest YAML is written as JSON so YAML 1.1 parsers cannot reinterpret strings.

fn parse_json(document: &str) -> Value {
    serde_json::from_str(document).expect("guest config is JSON")
}

fn network(mac_address: &str) -> CloudbaseInitNetwork {
    CloudbaseInitNetwork::ztp_interface(
        Ipv4Addr::new(172, 20, 0, 10),
        mac_address.to_string(),
        &helpers::test_network_v4(),
        None,
        None,
    )
}

// ============================================================================
// Tests
// ============================================================================

#[test]
fn generated_config_040_cloudbase_init_user_data_carries_the_nodes_name() {
    let config = CloudbaseInitConfig {
        set_hostname: "win01".to_string(),
        users: vec![],
        write_files: vec![],
        runcmd: vec![],
    };
    let output = config.render().expect("renders user-data");
    let body = output
        .strip_prefix("#cloud-config\n")
        .expect("user-data starts with the #cloud-config line");
    assert_eq!(
        parse_json(body),
        json!({ "set_hostname": "win01", "users": [], "write_files": [] })
    );
}

#[test]
fn config_addressing_003_cloudbase_init_network_config_ipv4() {
    let output = network("52:54:00:aa:bb:cc")
        .render()
        .expect("renders network-config");
    assert_eq!(
        parse_json(&output),
        json!({
            "version": 1,
            "config": [{
                "type": "physical",
                "name": "id0",
                "mac_address": "52:54:00:aa:bb:cc",
                "subnets": [{
                    "type": "static",
                    "address": "172.20.0.10",
                    "netmask": "255.255.255.0",
                    "gateway": "172.20.0.1",
                    "dns_nameservers": ["172.20.0.1"],
                }],
            }],
        })
    );
}

#[test]
fn config_addressing_004_cloudbase_init_network_config_dual_stack() {
    let v6 = helpers::test_network_v6();
    let net = CloudbaseInitNetwork::ztp_interface(
        Ipv4Addr::new(172, 20, 0, 10),
        "52:54:00:aa:bb:cc".to_string(),
        &helpers::test_network_v4(),
        Some("fd00::10".parse().expect("valid")),
        Some(&v6),
    );
    let output = net.render().expect("renders network-config");
    assert_eq!(
        parse_json(&output)["config"][0]["subnets"][1],
        json!({
            "type": "static6",
            "address": "fd00::10",
            "netmask": "64",
            "gateway": "fd00::1",
            "dns_nameservers": ["fd00::1"],
        })
    );
}

/// An all-digit MAC is a base-60 integer in YAML 1.1 when unquoted.
#[test]
fn generated_config_039_cloudbase_init_an_all_digit_mac_stays_a_string() {
    let output = network("52:54:00:21:47:08")
        .render()
        .expect("renders network-config");
    assert!(
        output.contains(r#""mac_address": "52:54:00:21:47:08""#),
        "MAC address must be a quoted string: {output}"
    );
}

#[test]
fn device_accounts_014_windows_owner_password_in_cloudbase_init() {
    for password in helpers::ALL_CHARACTER_PASSWORDS {
        let config = CloudbaseInitConfig {
            set_hostname: "win01".to_string(),
            users: vec![CloudbaseInitUser {
                name: "alice".to_string(),
                passwd: (*password).to_string(),
                groups: vec!["Administrators".to_string()],
                ssh_authorized_keys: vec!["ssh-rsa OWNERKEY1".to_string()],
            }],
            write_files: vec![],
            runcmd: vec![],
        };
        let output = config.render().expect("renders user-data");
        let body = output
            .strip_prefix("#cloud-config\n")
            .expect("user-data starts with the #cloud-config line");
        let parsed = parse_json(body);
        assert_eq!(parsed["users"].as_array().map(Vec::len), Some(1));
        assert_eq!(parsed["users"][0]["name"], "alice");
        assert_eq!(parsed["users"][0]["passwd"], *password);
        assert_eq!(output.matches(password).count(), 1);
    }
}
