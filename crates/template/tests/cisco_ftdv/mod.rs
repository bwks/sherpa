use std::net::Ipv4Addr;

use template::{CiscoFtdvZtpTemplate, CiscoFxosIpMode};

// ============================================================================
// Expected configs
// ============================================================================

// CiscoFtdvZtpTemplate serializes to JSON via serde.
// Field names are PascalCase via #[serde(rename)].
// CiscoFxosIpMode serializes lowercase. CiscoFxosFirewallMode serializes lowercase.
// manage_locally uses custom Yes/No serializer.
// Optional fields with skip_serializing_if are omitted when None.

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

const EXPECTED_MANAGE_LOCALLY_NO: &str = r#"{
  "EULA": "",
  "Hostname": "ftdv01",
  "AdminPassword": "",
  "FirewallMode": "routed",
  "ManageLocally": "No"
}"#;

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_manual_ipv4() {
    let t = CiscoFtdvZtpTemplate {
        eula: "accept".to_string(),
        hostname: "ftdv01".to_string(),
        admin_password: "Admin123!".to_string(),
        manage_locally: true,
        dns1: Some(Ipv4Addr::new(172, 20, 0, 1)),
        ipv4_mode: Some(CiscoFxosIpMode::Manual),
        ipv4_addr: Some(Ipv4Addr::new(172, 20, 0, 10)),
        ipv4_gw: Some(Ipv4Addr::new(172, 20, 0, 1)),
        ipv4_mask: Some(Ipv4Addr::new(255, 255, 255, 0)),
        ..Default::default()
    };
    let json = serde_json::to_string_pretty(&t).expect("serializes");
    assert_eq!(json, EXPECTED_MANUAL_IPV4);
}

#[test]
fn test_manage_locally_false() {
    let t = CiscoFtdvZtpTemplate {
        hostname: "ftdv01".to_string(),
        manage_locally: false,
        ..Default::default()
    };
    let json = serde_json::to_string_pretty(&t).expect("serializes");
    assert_eq!(json, EXPECTED_MANAGE_LOCALLY_NO);
}
