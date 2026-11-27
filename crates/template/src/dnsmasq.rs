use std::net::{Ipv4Addr, Ipv6Addr};

use askama::Template;

use shared::data::{Ztp, ZtpRecord};
use shared::konst::{
    BOOT_SERVER_LEASES_FILE, BOOT_SERVER_TFTP_ROOT, HTTP_PORT, SANDBOX_HOST_SUFFIX,
    SHERPA_DOMAIN_NAME,
};

/// The own management and MAC addresses of the sandbox of node `node_name`, which the boot
/// server names `<node_name>-sbx` and reserves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxHostRecord {
    pub node_name: String,
    pub ipv4: Ipv4Addr,
    pub ipv6: Option<Ipv6Addr>,
    pub mac: String,
}

#[derive(Template)]
#[template(path = "dnsmasq/config.jinja", ext = "txt")]
pub struct DnsmasqTemplate {
    pub tftp_server_ipv4: String,
    pub gateway_ipv4: String,
    pub dhcp_start: String,
    pub dhcp_end: String,
    pub gateway_ipv6: Option<String>,
    pub dhcp6_start: Option<String>,
    pub dhcp6_end: Option<String>,
    pub dns_ipv6: Option<String>,
    pub ztp_records: Vec<ZtpRecord>,
    pub sandbox_hosts: Vec<SandboxHostRecord>,
}

impl DnsmasqTemplate {
    /// The DHCP boot options a host is sent, as ready-made config lines. Only the methods that
    /// boot from the network have any: the others carry their config on media or a mount, and
    /// a node with ZTP off must not be pointed at a file nobody wrote.
    #[must_use]
    pub fn boot_options(&self, record: &ZtpRecord) -> Vec<String> {
        let tag = format!("dhcp-option=tag:{}-tag", record.node_name);
        match record.ztp {
            Ztp::Tftp => vec![format!("{tag},67,{}", record.config_file)],
            Ztp::Http => {
                let url = format!(
                    r#""http://{}:{HTTP_PORT}/{}""#,
                    self.tftp_server_ipv4, record.config_file
                );
                vec![format!("{tag},67,{url}"), format!("{tag},239,{url}")]
            }
            Ztp::Disabled
            | Ztp::CloudInit
            | Ztp::Cdrom
            | Ztp::Disk
            | Ztp::Ignition
            | Ztp::NoCloud
            | Ztp::Usb
            | Ztp::Volume => Vec::new(),
        }
    }
}
