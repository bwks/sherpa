use std::net::{Ipv4Addr, Ipv6Addr};

use askama::Template;

use shared::data::{Dns, NetworkV4, NetworkV6, OwnerUser, User};

#[derive(Template)]
#[template(path = "cisco/cisco_iosv.jinja", ext = "txt")]
pub struct CiscoIosvZtpTemplate {
    pub hostname: String,
    pub user: User,
    pub owner: Option<OwnerUser>,
    pub mgmt_interface: String,
    pub dns: Dns,
    pub mgmt_ipv4: NetworkV4,
    pub mgmt_ipv4_address: Option<Ipv4Addr>,
    pub mgmt_ipv6_address: Option<Ipv6Addr>,
    pub mgmt_ipv6: Option<NetworkV6>,
}

#[derive(Template)]
#[template(path = "cisco/cisco_iosvl2.jinja", ext = "txt")]
pub struct CiscoIosvl2ZtpTemplate {
    pub hostname: String,
    pub user: User,
    pub owner: Option<OwnerUser>,
    pub mgmt_interface: String,
    pub dns: Dns,
    pub mgmt_ipv4: NetworkV4,
    pub mgmt_ipv4_address: Option<Ipv4Addr>,
    pub mgmt_ipv6_address: Option<Ipv6Addr>,
    pub mgmt_ipv6: Option<NetworkV6>,
}

/// Cisco IOL's startup config. Its management address is always static: Docker gives the
/// container its address, and IOL takes it on `Ethernet0/0`. The L2 image (`switch`) routes its
/// management port.
#[derive(Template)]
#[template(path = "cisco/cisco_iol.jinja", ext = "txt")]
pub struct CiscoIolZtpTemplate {
    pub hostname: String,
    pub user: User,
    pub owner: Option<OwnerUser>,
    pub mgmt_interface: String,
    pub dns: Dns,
    pub mgmt_ipv4: NetworkV4,
    pub mgmt_ipv4_address: Ipv4Addr,
    pub mgmt_ipv6_address: Option<Ipv6Addr>,
    pub mgmt_ipv6: Option<NetworkV6>,
    pub switch: bool,
}
