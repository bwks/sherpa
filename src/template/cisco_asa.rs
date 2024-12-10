use rinja::Template;

use crate::data::{Dns, User};

#[derive(Template)]
#[template(path = "cisco/cisco_asa.jinja", ext = "txt")]
pub struct CiscoAsavZtpTemplate {
    pub hostname: String,
    pub users: Vec<User>,
    pub dns: Dns,
}
