use std::net::Ipv4Addr;

use askama::Template;

use template::CiscoIseZtpTemplate;

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_STATIC_IPV4: &str = "\
# Mandatory networking
hostname=ise01

ipv4_addr=172.20.0.10
ipv4_mask=255.255.255.0
ipv4_default_gw=172.20.0.1

# Optional IPv6

# DNS and domain
domain=lab.sherpa.local
primary_nameserver=172.20.0.1

# NTP
primary_ntpserver=time.cloudflare.com
# secondary_ntpserver=pool.ntp.org

# Timezone (TZ string as in Linux)
timezone=UTC

# services - optional
ssh=true
ers=true
openapi=true
pxgrid=true

# Admin credentials
username=sherpa
password=Everest1953!
# Optional SSH public key (ISE 3.2+)
public_key=ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ

# Skipping specific checks
SkipIcmpChecks=true
SkipDnsChecks=true
SkipNtpChecks=true";

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_static_ipv4() {
    let t = CiscoIseZtpTemplate {
        hostname: "ise01".to_string(),
        user: helpers::test_user(),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_STATIC_IPV4);
}
