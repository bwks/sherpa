use std::net::Ipv4Addr;

use askama::Template;

use crate::model::User;

#[derive(Template)]
#[template(
    source = r#"!
hostname {{ hostname }}
domain-name {{ crate::core::konst::SHERPA_DOMAIN_NAME }}
ip dns server-address {{ name_server }}
user admin group administrators password plaintext {{ crate::core::konst::SHERPA_PASSWORD }}
{%- for user in users %}
user {{ user.username }} {% if user.sudo %} group administrators{% endif %}{% if let Some(password) = user.password %} password plaintext {{ password }}{% endif %}
{%- endfor %}
!
ntp server pool.ntp.org minpoll 4 maxpoll 4 iburst
ntp enable
!
{%- for user in users %}
user {{ user.username }} authorized-key {{ user.ssh_public_key.algorithm }} {{ user.ssh_public_key.key }}
{%- endfor %}
!
ssh server vrf mgmt
vlan 1
interface mgmt
    no shutdown
    ip dhcp
!
https-server vrf mgmt
"#,
    ext = "txt"
)]
pub struct ArubaAoscxTemplate {
    pub hostname: String,
    pub users: Vec<User>,
    pub name_server: Ipv4Addr,
}
