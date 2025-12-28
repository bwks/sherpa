use askama::Template;

use data::ZtpRecord;
use konst::{
    SHERPA_BASE_DIR, SHERPA_DOMAIN_NAME, SHERPA_SSH_DIR, SHERPA_SSH_PRIVATE_KEY_FILE,
    SHERPA_USERNAME,
};

#[derive(Template)]
#[template(path = "ssh/ssh_config.jinja", ext = "txt")]
pub struct SshConfigTemplate {
    pub ztp_records: Vec<ZtpRecord>,
}
