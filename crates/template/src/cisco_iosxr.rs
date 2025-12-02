use askama::Template;

use data::{Dns, User};

#[derive(Template)]
#[template(path = "cisco/cisco_iosxr.jinja", ext = "txt")]
pub struct CiscoIosxrZtpTemplate {
    pub hostname: String,
    pub user: User,
    pub dns: Dns,
}
