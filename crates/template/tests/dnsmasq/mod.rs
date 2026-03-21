use std::net::Ipv4Addr;

use askama::Template;

use shared::data::{ZtpMethod, ZtpRecord};
use template::DnsmasqTemplate;

// ============================================================================
// Expected configs
// ============================================================================

// Note: the Jinja template produces extra blank lines around conditional blocks
// and a trailing newline after the last record. This expected string must match
// the exact rendering.
const EXPECTED_DNSMASQ_TFTP: &str = "# Logging
log-queries
log-dhcp
log-facility=/var/log/dnsmasq.log

port=53
server=172.20.0.1
domain=sherpa.lab.local

# DHCP
dhcp-leasefile=/opt/ztp/dnsmasq/dnsmasq.leases

# DHCP range for the subnet
dhcp-range=172.20.0.100,172.20.0.200,2m

# Set default gateway (Option 3) and option 150 (TFTP server IP)
dhcp-option=3,172.20.0.1 # Default Gateway
dhcp-option=6,172.20.0.1 # DNS Server
dhcp-option=15,sherpa.lab.local # Search Domain
dhcp-option=66,172.20.0.1 # TFTP Server
dhcp-option=150,172.20.0.1 # TFTP Server


# Ignore client identifier
dhcp-ignore-clid

# Enable TFTP
enable-tftp
tftp-root=/opt/ztp/tftp


# sw01
host-record=sw01,172.20.0.20
host-record=sw01.sherpa.lab.local,172.20.0.20
dhcp-host=52:54:00:11:22:33,172.20.0.20,sw01,,set:sw01-tag
dhcp-option=tag:sw01-tag,67,sw01.cfg
";

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_dnsmasq_tftp_ztp() {
    let t = DnsmasqTemplate {
        tftp_server_ipv4: "172.20.0.1".to_string(),
        gateway_ipv4: "172.20.0.1".to_string(),
        dhcp_start: "172.20.0.100".to_string(),
        dhcp_end: "172.20.0.200".to_string(),
        gateway_ipv6: None,
        dhcp6_start: None,
        dhcp6_end: None,
        dns_ipv6: None,
        ztp_records: vec![ZtpRecord {
            node_name: "sw01".to_string(),
            config_file: "sw01.cfg".to_string(),
            ipv4_address: Ipv4Addr::new(172, 20, 0, 20),
            ipv6_address: None,
            mac_address: "52:54:00:11:22:33".to_string(),
            ztp_method: ZtpMethod::Tftp,
            ssh_port: 22,
        }],
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_DNSMASQ_TFTP);
}
