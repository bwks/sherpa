use std::net::Ipv4Addr;

use template::build_srlinux_config;

use crate::helpers;

// ============================================================================
// Expected configs loaded from fixture files
// ============================================================================

const EXPECTED_STATIC_IPV4: &str = include_str!("fixtures/static_ipv4.json");

const EXPECTED_DHCP: &str = include_str!("fixtures/dhcp.json");

const EXPECTED_DUAL_STACK: &str = include_str!("fixtures/dual_stack.json");

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_static_ipv4() {
    let json = build_srlinux_config(
        "srl01",
        &helpers::test_user(),
        &helpers::test_dns(),
        &helpers::test_network_v4(),
        Some(Ipv4Addr::new(172, 20, 0, 10)),
        None,
        None,
    )
    .expect("builds config");
    assert_eq!(json, EXPECTED_STATIC_IPV4);
}

#[test]
fn test_dhcp() {
    let json = build_srlinux_config(
        "srl02",
        &helpers::test_user(),
        &helpers::test_dns(),
        &helpers::test_network_v4(),
        None,
        None,
        None,
    )
    .expect("builds config");
    assert_eq!(json, EXPECTED_DHCP);
}

#[test]
fn test_dual_stack() {
    let v6 = helpers::test_network_v6();
    let json = build_srlinux_config(
        "srl03",
        &helpers::test_user(),
        &helpers::test_dns(),
        &helpers::test_network_v4(),
        Some(Ipv4Addr::new(172, 20, 0, 10)),
        Some("fd00::10".parse().expect("valid")),
        Some(&v6),
    )
    .expect("builds config");
    assert_eq!(json, EXPECTED_DUAL_STACK);
}
