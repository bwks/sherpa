// use std::net::Ipv4Addr;

use rinja::Template;

use crate::data::User;

#[derive(Template)]
#[template(path = "juniper/juniper_vjunos.jinja", ext = "txt")]
pub struct JunipervJunosZtpTemplate {
    pub hostname: String,
    pub users: Vec<User>,
}
