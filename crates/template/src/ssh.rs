use askama::Template;

use shared::data::ZtpRecord;
use shared::konst::{SHERPA_DOMAIN_NAME, SHERPA_SSH_PRIVATE_KEY_FILE, SHERPA_USERNAME};

#[derive(Template)]
#[template(path = "ssh/ssh_config.jinja", ext = "txt")]
pub struct SshConfigTemplate {
    pub ztp_records: Vec<ZtpRecord>,
    pub proxy_user: String,
    pub server_ipv4: String,
}
