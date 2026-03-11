use std::net::Ipv4Addr;

use askama::Template;

use shared::data::{NetworkV4, User};

#[derive(Template)]
#[template(path = "frr/frr_config.jinja", ext = "txt")]
pub struct FrrZtpTemplate {
    pub hostname: String,
    pub mgmt_ipv4: NetworkV4,
    pub mgmt_ipv4_address: Option<Ipv4Addr>,
}

#[derive(Template)]
#[template(path = "frr/frr_daemons.jinja", ext = "txt")]
pub struct FrrDaemonsTemplate {}

#[derive(Template)]
#[template(path = "frr/frr_startup.jinja", ext = "txt")]
pub struct FrrStartupTemplate {
    pub hostname: String,
    pub user: User,
}
