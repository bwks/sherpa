use std::net::Ipv4Addr;

use askama::Template;

use template::MikrotikRouterosZtpTemplate;

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_STATIC_IPV4: &str = "\
/system identity set name=mikrotik01
/ip dns set servers=172.20.0.1
/user set 0 password=Everest1953!
:do { /user add name=sherpa password=Everest1953! group=full } on-error={}
/ip address add address=172.20.0.10/24 interface=ether1
/ip route add dst-address=0.0.0.0/0 gateway=172.20.0.1
/ip service set ssh disabled=no
/ip service set telnet disabled=yes
/ip service set www disabled=yes
/ip service set api disabled=yes
/ip ssh set strong-crypto=yes";

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_static_ipv4() {
    let t = MikrotikRouterosZtpTemplate {
        hostname: "mikrotik01".to_string(),
        user: helpers::test_user(),
        mgmt_interface: "ether1".to_string(),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_STATIC_IPV4);
}
