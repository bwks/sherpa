use rinja::Template;

use crate::data::{Dns, User};

#[derive(Template)]
#[template(path = "cisco/cisco_iosv.jinja", ext = "txt")]
pub struct CiscoIosvZtpTemplate {
    pub hostname: String,
    pub user: User,
    pub mgmt_interface: String,
    pub dns: Dns,
}

#[derive(Template)]
#[template(path = "cisco/cisco_iosvl2.jinja", ext = "txt")]
pub struct CiscoIosvl2ZtpTemplate {
    pub hostname: String,
    pub user: User,
    pub mgmt_interface: String,
    pub dns: Dns,
}
