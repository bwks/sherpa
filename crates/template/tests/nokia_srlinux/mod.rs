use std::net::Ipv4Addr;

use serde_json::{Value, json};

use template::build_srlinux_config;

use crate::helpers;

// ============================================================================
// Helpers
// ============================================================================

const SYSTEM: &str = "srl_nokia-system:system";
const AAA: &str = "srl_nokia-aaa:aaa";

/// The built config, parsed.
fn config(hostname: &str, password: &str, ipv4: bool, ipv6: bool) -> (String, Value) {
    let v6 = helpers::test_network_v6();
    let json = build_srlinux_config(
        hostname,
        &helpers::test_owner(password),
        &helpers::test_dns(),
        &helpers::test_network_v4(),
        ipv4.then(|| Ipv4Addr::new(172, 20, 0, 10)),
        ipv6.then(|| ("fd00::10".parse().expect("valid"), &v6)),
    )
    .expect("builds config");
    let value = serde_json::from_str(&json).expect("valid json");
    (json, value)
}

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_static_ipv4() {
    let (_, value) = config("srl01", "Rand0mPass-1", true, false);
    assert_eq!(
        value[SYSTEM]["srl_nokia-system-name:name"]["host-name"],
        "srl01"
    );
    assert_eq!(
        value[SYSTEM]["srl_nokia-dns:dns-instance"],
        json!([{
            "name": "mgmt",
            "network-instance": "mgmt",
            "server-list": ["172.20.0.1"],
        }])
    );
    let subinterface = &value["srl_nokia-interfaces:interface"][0]["subinterface"][0];
    assert_eq!(value["srl_nokia-interfaces:interface"][0]["name"], "mgmt0");
    assert_eq!(
        subinterface["ipv4"],
        json!({
            "admin-state": "enable",
            "address": [{"ip-prefix": "172.20.0.10/24"}],
        })
    );
    assert_eq!(
        subinterface["ipv6"],
        json!({
            "admin-state": "enable",
            "srl_nokia-interfaces-ip-dhcp:dhcp-client": {},
        })
    );
}

#[test]
fn test_dhcp() {
    let (_, value) = config("srl02", "Rand0mPass-1", false, false);
    assert_eq!(
        value[SYSTEM]["srl_nokia-system-name:name"]["host-name"],
        "srl02"
    );
    let subinterface = &value["srl_nokia-interfaces:interface"][0]["subinterface"][0];
    assert_eq!(
        subinterface["ipv4"],
        json!({
            "admin-state": "enable",
            "srl_nokia-interfaces-ip-dhcp:dhcp-client": {},
        })
    );
    assert!(subinterface["ipv4"].get("address").is_none());
}

#[test]
fn test_dual_stack() {
    let (_, value) = config("srl03", "Rand0mPass-1", true, true);
    assert_eq!(
        value[SYSTEM]["srl_nokia-system-name:name"]["host-name"],
        "srl03"
    );
    let subinterface = &value["srl_nokia-interfaces:interface"][0]["subinterface"][0];
    assert_eq!(
        subinterface["ipv4"]["address"],
        json!([{"ip-prefix": "172.20.0.10/24"}])
    );
    assert_eq!(
        subinterface["ipv6"]["address"],
        json!([{"ip-prefix": "fd00::10/64"}])
    );
    assert!(
        subinterface["ipv6"]
            .get("srl_nokia-interfaces-ip-dhcp:dhcp-client")
            .is_none()
    );
}

#[test]
fn test_management_network_instance_and_acl() {
    let (_, value) = config("srl01", "Rand0mPass-1", true, false);
    let instance = &value["srl_nokia-network-instance:network-instance"][0];
    assert_eq!(instance["name"], "mgmt");
    assert_eq!(instance["interface"], json!([{"name": "mgmt0.0"}]));
    assert!(value["srl_nokia-acl:acl"].is_object());
    assert_eq!(
        value[SYSTEM]["control-plane-traffic"]["input"]["srl_nokia-acl:acl"]["acl-filter"],
        json!([
            {"name": "cpm", "type": "ipv4"},
            {"name": "cpm", "type": "ipv6"},
        ])
    );
}

#[test]
fn test_only_admin_and_the_owner_have_accounts() {
    let (_, value) = config("srl01", "Rand0mPass-1", true, false);
    let authentication = &value[SYSTEM][AAA]["authentication"];
    assert_eq!(authentication["authentication-method"], json!(["local"]));
    assert_eq!(
        authentication["admin-user"],
        json!({"password": "Rand0mPass-1"})
    );
    assert_eq!(
        authentication["user"],
        json!([{
            "username": "alice",
            "password": "Rand0mPass-1",
            "ssh-key": ["ssh-rsa OWNERKEY1", "ssh-ed25519 OWNERKEY2"],
            "role": ["srl-admin"],
        }])
    );
}

#[test]
fn test_password_is_intact_for_admin_and_owner() {
    for password in helpers::ALL_CHARACTER_PASSWORDS {
        let (json, value) = config("srl01", password, true, false);
        let authentication = &value[SYSTEM][AAA]["authentication"];
        assert_eq!(authentication["admin-user"]["password"], *password);
        assert_eq!(authentication["user"][0]["password"], *password);
        assert_eq!(authentication["user"].as_array().map(Vec::len), Some(1));
        assert_eq!(json.matches(password).count(), 2);
    }
}
