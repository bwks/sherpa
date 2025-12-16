use std::net::Ipv4Addr;

use askama::Template;

use data::{Dns, NetworkV4, User};

#[derive(Template)]
#[template(path = "cisco/cisco_iosxe.jinja", ext = "txt")]
pub struct CiscoIosXeZtpTemplate {
    pub hostname: String,
    pub user: User,
    pub mgmt_interface: String,
    pub dns: Dns,
    pub license_boot_command: Option<String>,
    pub mgmt_ipv4: NetworkV4,
    pub mgmt_ipv4_address: Option<Ipv4Addr>,
}
