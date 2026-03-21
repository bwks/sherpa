use std::net::Ipv4Addr;

use askama::Template;

use template::PaloAltoPanosZtpTemplate;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_INIT_IPV4: &str = "\
type=static
ip-address=172.20.0.10
default-gateway=172.20.0.1
netmask=255.255.255.0
hostname=panos01
dns-primary=172.20.0.1
dns-secondary=8.8.8.8";

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_panos_init_static_ipv4() {
    let t = PaloAltoPanosZtpTemplate {
        hostname: "panos01".to_string(),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
        mgmt_netmask: Ipv4Addr::new(255, 255, 255, 0),
        mgmt_gateway: Ipv4Addr::new(172, 20, 0, 1),
        dns_primary: Ipv4Addr::new(172, 20, 0, 1),
        dns_secondary: Ipv4Addr::new(8, 8, 8, 8),
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_INIT_IPV4);
}
