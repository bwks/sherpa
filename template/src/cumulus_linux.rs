use askama::Template;

use data::{Dns, User};

#[derive(Template)]
#[template(path = "cumulus/cumulus_linux.jinja", ext = "txt")]
pub struct CumulusLinuxZtpTemplate {
    pub hostname: String,
    pub user: User,
    pub dns: Dns,
}
