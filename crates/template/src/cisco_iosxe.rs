use askama::Template;

use data::{Dns, User};

#[derive(Template)]
#[template(path = "cisco/cisco_iosxe.jinja", ext = "txt")]
pub struct CiscoIosXeZtpTemplate {
    pub hostname: String,
    pub user: User,
    pub mgmt_interface: String,
    pub dns: Dns,
}
