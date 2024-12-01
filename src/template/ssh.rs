use askama::Template;

use crate::core::konst::{SHERPA_CONFIG_DIR, SHERPA_SSH_PRIVATE_KEY_FILE, SHERPA_USERNAME};
use crate::data::DeviceIp;

#[derive(Template)]
#[template(
    source = r#"Host *
    User {{ SHERPA_USERNAME }}
    IdentityFile {{ SHERPA_CONFIG_DIR }}/{{ SHERPA_SSH_PRIVATE_KEY_FILE }}
    IdentitiesOnly yes
    StrictHostKeyChecking no
    UserKnownHostsFile /dev/null
    PubkeyAcceptedAlgorithms +ssh-rsa
    HostkeyAlgorithms +ssh-rsa
    KexAlgorithms +diffie-hellman-group-exchange-sha1,diffie-hellman-group14-sha1
{%- for host in hosts %}
Host {{ host.name }}
    HostName {{ host.ip_address }}
{%- endfor %}
"#,
    ext = "txt"
)]
pub struct SshConfigTemplate {
    pub hosts: Vec<DeviceIp>,
}
