use askama::Template;

use crate::core::{SHERPA_CONFIG_DIR, SHERPA_SSH_PRIVATE_KEY_FILE, SHERPA_USERNAME};
use crate::data::DeviceConnection;

#[derive(Template)]
#[template(path = "ssh/ssh_config.jinja", ext = "txt")]
pub struct SshConfigTemplate {
    pub hosts: Vec<DeviceConnection>,
}
