use std::net::{Ipv4Addr, Ipv6Addr};

use askama::Template;

use shared::data::{NetworkV6, OwnerUser};

/// Renders the PAN-OS `init-cfg.txt` bootstrap file.
/// Network and hostname settings only — user configuration
/// is handled by `bootstrap.xml` (see `PaloAltoPanosBootstrapTemplate`).
#[derive(Template)]
#[template(path = "paloalto/paloalto_panos_init.jinja", ext = "txt")]
pub struct PaloAltoPanosZtpTemplate {
    pub hostname: String,
    pub mgmt_ipv4_address: Ipv4Addr,
    pub mgmt_ipv6_address: Option<Ipv6Addr>,
    pub mgmt_ipv6: Option<NetworkV6>,
    pub mgmt_netmask: Ipv4Addr,
    pub mgmt_gateway: Ipv4Addr,
    pub dns_primary: Ipv4Addr,
    pub dns_secondary: Ipv4Addr,
}

/// Renders the PAN-OS `bootstrap.xml` configuration file.
/// Based on the PA-VM 11.1 running-config structure.
/// Configures the `admin` account and the lab owner's, both with the owner's device password as
/// a SHA-256-crypt hash, enables SSH on the management interface, and disables telnet.
#[derive(Template)]
#[template(path = "paloalto/paloalto_panos_bootstrap.jinja", escape = "xml")]
pub struct PaloAltoPanosBootstrapTemplate {
    pub hostname: String,
    pub owner: OwnerUser,
    pub mgmt_ipv4_address: Ipv4Addr,
    pub mgmt_ipv6_address: Option<Ipv6Addr>,
    pub mgmt_ipv6: Option<NetworkV6>,
    pub mgmt_netmask: Ipv4Addr,
    pub mgmt_gateway: Ipv4Addr,
    pub dns_primary: Ipv4Addr,
}
