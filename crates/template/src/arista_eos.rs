use askama::Template;
use std::net::{Ipv4Addr, Ipv6Addr};

use shared::data::{Dns, NetworkV4, NetworkV6, OwnerUser};

#[derive(Template)]
#[template(path = "arista/arista_eos_vm.jinja", ext = "txt")]
pub struct AristaEosVmZtpTemplate {
    pub hostname: String,
    pub owner: OwnerUser,
    pub dns: Dns,
    pub mgmt_ipv4: NetworkV4,
    pub mgmt_ipv4_address: Option<Ipv4Addr>,
    pub mgmt_ipv6_address: Option<Ipv6Addr>,
    pub mgmt_ipv6: Option<NetworkV6>,
}

#[derive(Template)]
#[template(path = "arista/arista_eos_container.jinja", ext = "txt")]
pub struct AristaEosContainerZtpTemplate {
    pub hostname: String,
    pub owner: OwnerUser,
    pub dns: Dns,
    pub mgmt_ipv4: NetworkV4,
    pub mgmt_ipv4_address: Option<Ipv4Addr>,
    pub mgmt_ipv6_address: Option<Ipv6Addr>,
    pub mgmt_ipv6: Option<NetworkV6>,
}
