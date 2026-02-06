use std::net::Ipv4Addr;

use askama::Template;

use data::{Dns, NetworkV4, User};
use shared::konst::SHERPA_PASSWORD;

#[derive(Template)]
#[template(path = "aruba/aruba_aoscx.jinja", ext = "txt")]
pub struct ArubaAoscxTemplate {
    pub hostname: String,
    pub user: User,
    pub dns: Dns,
    pub mgmt_ipv4: NetworkV4,
    pub mgmt_ipv4_address: Option<Ipv4Addr>,
}
