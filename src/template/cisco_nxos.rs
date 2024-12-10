use rinja::Template;

use crate::data::{Dns, User};

#[derive(Template)]
#[template(path = "cisco/cisco_nxos.jinja", ext = "txt")]
pub struct CiscoNxosZtpTemplate {
    pub hostname: String,
    pub users: Vec<User>,
    pub dns: Dns,
}
