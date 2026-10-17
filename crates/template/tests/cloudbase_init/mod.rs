use std::net::Ipv4Addr;

use serde_json::{Value, json};
use template::{CloudbaseInitConfig, CloudbaseInitNetwork};

use crate::helpers;

// Guest YAML is written as JSON so YAML 1.1 parsers cannot reinterpret strings.

fn parse_json(document: &str) -> Value {
    serde_json::from_str(document).expect("guest config is JSON")
}

fn network(mac_address: &str) -> CloudbaseInitNetwork {
    CloudbaseInitNetwork::ztp_interface(
        Ipv4Addr::new(172, 20, 0, 10),
        mac_address.to_string(),
        helpers::test_network_v4(),
        None,
        None,
    )
}

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_cloudbase_config() {
    let config = CloudbaseInitConfig {
        set_hostname: "win01".to_string(),
        users: vec![],
        write_files: vec![],
        runcmd: vec![],
    };
    let output = config.to_string().expect("renders user-data");
    let body = output
        .strip_prefix("#cloud-config\n")
        .expect("user-data starts with the #cloud-config line");
    assert_eq!(
        parse_json(body),
        json!({ "set_hostname": "win01", "users": [], "write_files": [] })
    );
}

#[test]
fn test_cloudbase_network_ipv4() {
    let output = network("52:54:00:aa:bb:cc")
        .to_string()
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
fn test_cloudbase_network_dual_stack() {
    let v6 = helpers::test_network_v6();
    let net = CloudbaseInitNetwork::ztp_interface(
        Ipv4Addr::new(172, 20, 0, 10),
        "52:54:00:aa:bb:cc".to_string(),
        helpers::test_network_v4(),
        Some("fd00::10".parse().expect("valid")),
        Some(&v6),
    );
    let output = net.to_string().expect("renders network-config");
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
fn test_all_digit_mac_address_is_a_quoted_string() {
    let output = network("52:54:00:21:47:08")
        .to_string()
        .expect("renders network-config");
    assert!(
        output.contains(r#""mac_address": "52:54:00:21:47:08""#),
        "MAC address must be a quoted string: {output}"
    );
}
