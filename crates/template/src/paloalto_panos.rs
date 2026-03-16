use std::net::Ipv4Addr;

use askama::Template;

use shared::data::User;

/// Renders the PAN-OS `init-cfg.txt` bootstrap file.
/// Network and hostname settings only — user configuration
/// is handled by `bootstrap.xml` (see `PaloAltoPanosBootstrapTemplate`).
#[derive(Template)]
#[template(path = "paloalto/paloalto_panos_init.jinja", ext = "txt")]
pub struct PaloAltoPanosZtpTemplate {
    pub hostname: String,
    pub mgmt_ipv4_address: Ipv4Addr,
    pub mgmt_netmask: Ipv4Addr,
    pub mgmt_gateway: Ipv4Addr,
    pub dns_primary: Ipv4Addr,
    pub dns_secondary: Ipv4Addr,
}

/// Renders the PAN-OS `bootstrap.xml` configuration file.
/// Configures the admin user account with password hash and SSH key,
/// enables SSH on the management interface, and disables telnet/HTTP.
#[derive(Template)]
#[template(path = "paloalto/paloalto_panos_bootstrap.jinja", ext = "txt")]
pub struct PaloAltoPanosBootstrapTemplate {
    pub hostname: String,
    pub user: User,
    pub password_hash: String,
    pub panos_version: String,
    pub mgmt_ipv4_address: Ipv4Addr,
    pub mgmt_netmask: Ipv4Addr,
    pub mgmt_gateway: Ipv4Addr,
    pub dns_primary: Ipv4Addr,
}

pub const PANOS_DEFAULT_VERSION: &str = "10.1.0";
