use askama::Template;

pub struct DeviceIp {
    pub name: String,
    pub ip_address: String,
}

#[derive(Template)]
#[template(
    source = r#"Host *
    User {{ crate::core::konst::SHERPA_USERNAME }}
    IdentityFile {{ crate::core::konst::SHERPA_CONFIG_DIR }}/{{ crate::core::konst::SHERPA_SSH_PRIVATE_KEY_FILE }}
    IdentitiesOnly yes
    StrictHostKeyChecking no
    UserKnownHostsFile /dev/null
    PubkeyAcceptedAlgorithms +ssh-rsa
    HostkeyAlgorithms +ssh-rsa
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
