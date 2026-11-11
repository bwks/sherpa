use askama::Template;

use shared::data::{Ztp, ZtpRecord};
use shared::konst::{
    DNSMASQ_DIR, DNSMASQ_LEASES_FILE, NODE_CONFIGS_DIR, SHERPA_DOMAIN_NAME, TFTP_DIR, ZTP_DIR,
};

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
                    r#""http://{}:8080/{NODE_CONFIGS_DIR}/{}""#,
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
