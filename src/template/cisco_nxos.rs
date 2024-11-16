use std::net::Ipv4Addr;

use askama::Template;

use crate::model::User;

#[derive(Template)]
#[template(
    source = r#"!
feature bash-shell
feature nxapi
feature scp-server
!
hostname {{ hostname }}
!
username admin password 0 {{ crate::core::konst::SHERPA_PASSWORD }} role network-admin
{%- for user in users %}
{%-   if let Some(password) = user.password %}
username {{ user.username }} password 0 {{ password }} 
{%-   endif %}
{%-   if user.sudo %}
username {{ user.username }} role network-admin
{%-   endif %}
username {{ user.username }} sshkey {{ user.ssh_public_key.algorithm }} {{ user.ssh_public_key.key }}
{%- endfor %}
!
ip name-server {{ name_server }}
!
line vty
  exec-timeout 0
!
interface mgmt0
  ip address dhcp
  no shutdown
!
"#,
    ext = "txt"
)]
pub struct CiscoNxosZtpTemplate {
    pub hostname: String,
    pub users: Vec<User>,
    pub name_server: Ipv4Addr,
}
