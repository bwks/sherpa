use askama::Template;
use std::net::Ipv4Addr;

use data::{Dns, NetworkV4, User};

#[derive(Template)]
#[template(path = "arista/arista_veos.jinja", ext = "txt")]
pub struct AristaVeosZtpTemplate {
    pub hostname: String,
    pub user: User,
    pub dns: Dns,
    pub mgmt_ipv4: NetworkV4,
    pub mgmt_ipv4_address: Option<Ipv4Addr>,
}
