use std::net::Ipv4Addr;

use askama::Template;

use shared::data::{Dns, NetworkV4, User};

#[derive(Template)]
#[template(path = "cisco/cisco_iosv.jinja", ext = "txt")]
pub struct CiscoIosvZtpTemplate {
    pub hostname: String,
    pub user: User,
    pub mgmt_interface: String,
    pub dns: Dns,
    pub mgmt_ipv4: NetworkV4,
    pub mgmt_ipv4_address: Option<Ipv4Addr>,
}

#[derive(Template)]
#[template(path = "cisco/cisco_iosvl2.jinja", ext = "txt")]
pub struct CiscoIosvl2ZtpTemplate {
    pub hostname: String,
    pub user: User,
    pub mgmt_interface: String,
    pub dns: Dns,
    pub mgmt_ipv4: NetworkV4,
    pub mgmt_ipv4_address: Option<Ipv4Addr>,
}
