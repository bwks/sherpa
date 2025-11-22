use askama::Template;

use data::User;

#[derive(Template)]
#[template(path = "sonic/ztp_user.jinja", ext = "txt")]
pub struct SonicLinuxZtpTemplate {
    pub user: User,
}
