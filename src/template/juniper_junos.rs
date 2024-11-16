// use std::net::Ipv4Addr;

use askama::Template;

use crate::model::User;

#[derive(Template)]
#[template(
    source = r#"!
{%- for user in users %}
set system login user {{ user.username }} {% if user.sudo %}class super-user{% endif %}
set system login user {{ user.username }} authentication {{ user.ssh_public_key.algorithm }} "{{ user.ssh_public_key.algorithm }} {{ user.ssh_public_key.key }}"
{%-   if let Some(password) = user.password %}
set system login user {{ user.username }} authentication plain-text-password {{ password }}
{%-   endif %}
{%- endfor %}
set system root-authentication plain-text-password "{{ crate::core::konst::SHERPA_PASSWORD }}"
set system host-name {{ hostname }}
set system services ssh root-login allow
set system services netconf ssh
commit and-quit
"#,
    ext = "txt"
)]
pub struct JunipervJunosZtpTemplate {
    pub hostname: String,
    pub users: Vec<User>,
}
