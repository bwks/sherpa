use serde_derive::{Deserialize, Serialize};

use askama::Template;

use crate::model::User;

#[derive(Template)]
#[template(
    source = r#"#cloud-config
hostname: {{ hostname }}
fqdn: {{ hostname }}.{{ crate::core::konst::SHERPA_DOMAIN_NAME }}
{%- if password_auth %}
ssh_pwauth: True
{%- endif %}
users:
  {%- for user in users %}
  - name: {{ user.username }}
    {%- if let Some(password) = user.password %}
    plain_text_passwd: {{ password }}
    lock_passwd: false
    {%- endif %}
    ssh_authorized_keys:
      - {{ user.ssh_public_key.algorithm }} {{ user.ssh_public_key.key }}
    sudo: "ALL=(ALL) NOPASSWD:ALL"
    {%- if user.sudo %}
    groups: sudo
    {%- endif %}
    shell: /bin/bash
  {%- endfor %}
"#,
    ext = "yml"
)]
pub struct CloudInitTemplate {
    pub hostname: String,
    pub users: Vec<User>,
    pub password_auth: bool,
}

#[derive(Serialize, Deserialize, Debug)]
struct CloudConfig {
    hostname: String,
    fqdn: String,
    users: Vec<CloudInitUser>,
}

#[derive(Serialize, Deserialize, Debug)]
struct CloudInitUser {
    name: String,
    plain_text_passwd: String,
    lock_passwd: bool,
    ssh_authorized_keys: Vec<String>,
    sudo: String,
    groups: Vec<String>,
    shell: String,
}
