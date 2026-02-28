use std::net::Ipv4Addr;

use askama::Template;

use shared::data::{Dns, NetworkV4, User};

#[derive(Template)]
#[template(path = "cisco/cisco_asa.jinja", ext = "txt")]
pub struct CiscoAsavZtpTemplate {
    pub hostname: String,
    pub user: User,
    pub dns: Dns,
    pub mgmt_ipv4: NetworkV4,
    pub mgmt_ipv4_address: Option<Ipv4Addr>,
}
