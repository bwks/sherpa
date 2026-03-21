use std::net::Ipv4Addr;

use template::{CloudbaseInitConfig, CloudbaseInitNetwork};

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_CLOUDBASE_CONFIG: &str = "#cloud-config
set_hostname: win01
users: []
write_files: []
";

const EXPECTED_NETWORK_IPV4: &str = "version: 1
config:
- type: physical
  name: id0
  mac_address: 52:54:00:aa:bb:cc
  subnets:
  - type: static
    address: 172.20.0.10
    netmask: 255.255.255.0
    gateway: 172.20.0.1
    dns_nameservers:
    - 172.20.0.1
";

const EXPECTED_NETWORK_DUAL_STACK: &str = "version: 1
config:
- type: physical
  name: id0
  mac_address: 52:54:00:aa:bb:cc
  subnets:
  - type: static
    address: 172.20.0.10
    netmask: 255.255.255.0
    gateway: 172.20.0.1
    dns_nameservers:
    - 172.20.0.1
  - type: static6
    address: fd00::10
    netmask: '64'
    gateway: fd00::1
    dns_nameservers:
    - fd00::1
";

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
    let output = config.to_string().expect("renders yaml");
    assert_eq!(output, EXPECTED_CLOUDBASE_CONFIG);
}

#[test]
fn test_cloudbase_network_ipv4() {
    let net = CloudbaseInitNetwork::ztp_interface(
        Ipv4Addr::new(172, 20, 0, 10),
        "52:54:00:aa:bb:cc".to_string(),
        helpers::test_network_v4(),
        None,
        None,
    );
    let output = net.to_string().expect("renders yaml");
    assert_eq!(output, EXPECTED_NETWORK_IPV4);
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
    let output = net.to_string().expect("renders yaml");
    assert_eq!(output, EXPECTED_NETWORK_DUAL_STACK);
}
