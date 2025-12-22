use std::net::Ipv4Addr;

use askama::Template;

use data::{Dns, NetworkV4, User};

#[derive(Template)]
#[template(path = "cisco/cisco_ise.jinja", ext = "txt")]
pub struct CiscoIseZtpTemplate {
    pub hostname: String,
    pub user: User,
    pub dns: Dns,
    pub mgmt_ipv4: NetworkV4,
    pub mgmt_ipv4_address: Ipv4Addr,
}
