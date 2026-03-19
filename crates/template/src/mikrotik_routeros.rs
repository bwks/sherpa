use std::net::{Ipv4Addr, Ipv6Addr};

use askama::Template;

use shared::data::{Dns, NetworkV4, NetworkV6, User};

#[derive(Template)]
#[template(path = "mikrotik/mikrotik_routeros.jinja", ext = "txt")]
pub struct MikrotikRouterosZtpTemplate {
    pub hostname: String,
    pub user: User,
    pub mgmt_interface: String,
    pub dns: Dns,
    pub mgmt_ipv4: NetworkV4,
    pub mgmt_ipv4_address: Option<Ipv4Addr>,
    pub mgmt_ipv6_address: Option<Ipv6Addr>,
    pub mgmt_ipv6: Option<NetworkV6>,
}
