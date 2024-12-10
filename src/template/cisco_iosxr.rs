use rinja::Template;

use crate::data::{Dns, User};

#[derive(Template)]
#[template(path = "cisco/cisco_iosxr.jinja", ext = "txt")]
pub struct CiscoIosxrZtpTemplate {
    pub hostname: String,
    pub users: Vec<User>,
    pub dns: Dns,
}
