use std::net::Ipv4Addr;

use askama::Template;

use data::{NetworkV4, User};

#[derive(Template)]
#[template(path = "juniper/juniper_junos.jinja", ext = "txt")]
pub struct JunipervJunosZtpTemplate {
    pub hostname: String,
    pub user: User,
    pub mgmt_interface: String,
    pub mgmt_ipv4: NetworkV4,
    pub mgmt_ipv4_address: Option<Ipv4Addr>,
}
