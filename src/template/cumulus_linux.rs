use rinja::Template;

use crate::data::{Dns, User};

#[derive(Template)]
#[template(path = "cumulus/cumulus_linux.jinja", ext = "txt")]
pub struct CumulusLinuxZtpTemplate {
    pub hostname: String,
    pub users: Vec<User>,
    pub dns: Dns,
}
