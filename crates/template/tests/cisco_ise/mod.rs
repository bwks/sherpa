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
username=alice
password=PASSWORD
# Optional SSH public key (ISE 3.2+)
public_key=ssh-rsa OWNERKEY1

# Skipping specific checks
SkipIcmpChecks=true
SkipDnsChecks=true
SkipNtpChecks=true";

// ============================================================================
// Tests
// ============================================================================

fn ise(password: &str) -> CiscoIseZtpTemplate {
    CiscoIseZtpTemplate {
        hostname: "ise01".to_string(),
        owner: helpers::test_owner(password),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
}

#[test]
fn test_static_ipv4() {
    let output = ise("Rand0mPass-1").render().expect("template renders");
    assert_eq!(
        output,
        EXPECTED_STATIC_IPV4.replace("PASSWORD", "Rand0mPass-1")
    );
}

#[test]
fn test_password_is_intact() {
    for password in helpers::ALL_CHARACTER_PASSWORDS {
        let output = ise(password).render().expect("template renders");
        assert!(output.contains(&format!("\npassword={password}\n")));
        assert_eq!(output.matches(password).count(), 1);
    }
}
