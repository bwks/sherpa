use askama::Template;

use data::{Dns, User};

#[derive(Template)]
#[template(path = "cisco/cisco_nxos.jinja", ext = "txt")]
pub struct CiscoNxosZtpTemplate {
    pub hostname: String,
    pub user: User,
    pub dns: Dns,
}
