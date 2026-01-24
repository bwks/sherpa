use askama::Template;

use data::{ZtpMethod, ZtpRecord};
use konst::{
    DNSMASQ_DIR, DNSMASQ_LEASES_FILE, NODE_CONFIGS_DIR, SHERPA_DOMAIN_NAME, TFTP_DIR, ZTP_DIR,
};

#[derive(Template)]
#[template(path = "dnsmasq/config.jinja", ext = "txt")]
pub struct DnsmasqTemplate {
    pub tftp_server_ipv4: String,
    pub gateway_ipv4: String,
    pub dhcp_start: String,
    pub dhcp_end: String,
    pub ztp_records: Vec<ZtpRecord>,
}
