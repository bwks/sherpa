use askama::Template;

use data::DeviceConnection;
use konst::{SHERPA_CONFIG_DIR, SHERPA_SSH_PRIVATE_KEY_FILE, SHERPA_USERNAME};

#[derive(Template)]
#[template(path = "ssh/ssh_config.jinja", ext = "txt")]
pub struct SshConfigTemplate {
    pub hosts: Vec<DeviceConnection>,
}
