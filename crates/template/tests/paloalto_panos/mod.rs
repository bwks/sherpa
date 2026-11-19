use std::net::Ipv4Addr;

use askama::Template;

use template::{PaloAltoPanosBootstrapTemplate, PaloAltoPanosZtpTemplate};

use crate::helpers;

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

fn bootstrap(hash: &str) -> PaloAltoPanosBootstrapTemplate {
    PaloAltoPanosBootstrapTemplate {
        hostname: "panos01".to_string(),
        owner: helpers::test_owner(hash),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
        mgmt_netmask: Ipv4Addr::new(255, 255, 255, 0),
        mgmt_gateway: Ipv4Addr::new(172, 20, 0, 1),
        dns_primary: Ipv4Addr::new(172, 20, 0, 1),
    }
}

/// The text between `<users>` and `</users>`.
fn users_block(output: &str) -> &str {
    let start = output.find("<users>").expect("has users");
    let end = output.find("</users>").expect("has users end");
    &output[start..end]
}

#[test]
fn test_bootstrap_users_are_admin_and_the_owner() {
    let output = bootstrap("$5$salt$HASH")
        .render()
        .expect("template renders");
    // Only the owner has keys.
    let expected = r#"<users>
      <entry name="admin">
        <phash>$5$salt$HASH</phash>
        <permissions>
          <role-based>
            <superuser>yes</superuser>
          </role-based>
        </permissions>
      </entry>
      <entry name="alice">
        <phash>$5$salt$HASH</phash>
        <permissions>
          <role-based>
            <superuser>yes</superuser>
          </role-based>
        </permissions>
        <public-key>OWNERKEY1</public-key>
        <public-key>OWNERKEY2</public-key>
      </entry>
    "#;
    assert_eq!(users_block(&output), expected);
}
