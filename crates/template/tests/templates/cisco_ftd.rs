use std::net::Ipv4Addr;

use template::{CiscoFtdZtpTemplate, CiscoFxosFirewallMode};

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

// CiscoFtdZtpTemplate serializes to JSON via serde.
// Field names are PascalCase via #[serde(rename)].
// The IP modes and CiscoFxosFirewallMode serialize lowercase.
// manage_locally serializes as Yes or No.
// The IPv6 keys are omitted without both an IPv6 address and an IPv6 network.

const EXPECTED_MANUAL_IPV4: &str = r#"{
  "EULA": "accept",
  "Hostname": "ftdv01",
  "AdminPassword": "Admin123!",
  "FirewallMode": "routed",
  "DNS1": "172.20.0.1",
  "IPv4Mode": "manual",
  "IPv4Addr": "172.20.0.10",
  "IPv4Gw": "172.20.0.1",
  "IPv4Mask": "255.255.255.0",
  "ManageLocally": "Yes"
}"#;

// ============================================================================
// Tests
// ============================================================================

#[test]
fn generated_config_005_cisco_ftdv_day0_json() {
    let t = CiscoFtdZtpTemplate {
        eula: "accept".to_string(),
        hostname: "ftdv01".to_string(),
        admin_password: "Admin123!".to_string(),
        firewall_mode: CiscoFxosFirewallMode::Routed,
        dns: Ipv4Addr::new(172, 20, 0, 1),
        manage_locally: true,
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    };
    let json = serde_json::to_string_pretty(&t).expect("serializes");
    assert_eq!(json, EXPECTED_MANUAL_IPV4);
}

#[test]
fn device_accounts_002_cisco_ftdv_admin() {
    for password in helpers::ALL_CHARACTER_PASSWORDS {
        let t = CiscoFtdZtpTemplate {
            eula: "accept".to_string(),
            hostname: "ftdv01".to_string(),
            admin_password: (*password).to_string(),
            firewall_mode: CiscoFxosFirewallMode::Routed,
            dns: Ipv4Addr::new(172, 20, 0, 1),
            manage_locally: true,
            mgmt_ipv4: helpers::test_network_v4(),
            mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
            mgmt_ipv6_address: None,
            mgmt_ipv6: None,
        };
        let json = serde_json::to_string_pretty(&t).expect("serializes");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(parsed["AdminPassword"], *password);
        assert!(json.contains(&format!(r#""AdminPassword": "{password}""#)));
    }
}
