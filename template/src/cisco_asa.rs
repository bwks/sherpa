use askama::Template;
use data::Dns;
use data::User;

#[derive(Template)]
#[template(path = "cisco/cisco_asa.jinja", ext = "txt")]
pub struct CiscoAsavZtpTemplate {
    pub hostname: String,
    pub user: User,
    pub dns: Dns,
}
