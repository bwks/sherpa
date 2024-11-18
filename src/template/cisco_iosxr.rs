use askama::Template;

use crate::data::{Dns, User};

#[derive(Template)]
#[template(
    source = r#"!
hostname {{ hostname }}
domain name {{ dns.domain }}
{%- for server in dns.name_servers %}
domain name-server {{ server.ipv4_address }}
{%- endfor %}
username admin
 group root-lr
 group cisco-support
 secret 0 {{ crate::core::konst::SHERPA_PASSWORD }}
!
{%- for user in users %}
username {{ user.username }}
{%-   if user.sudo %}
 group netadmin
{%-   endif %}
{%-   if let Some(password) = user.password %}
 password 0 {{ password }} 
{%-   endif %}
{%- endfor %}
!
aaa authorization exec default local
aaa authentication login default local
!
netconf-yang agent
 ssh
!
interface MgmtEth0/RP0/CPU0/0
 ipv4 address dhcp
 no shutdown
!
netconf agent tty
 session timeout 5
!
ssh server logging
!
{%- for user in users %}
ssh server username {{ user.username }}
 keystring {{ user.ssh_public_key.algorithm }} {{ user.ssh_public_key.key }}
{%- endfor %}
!
ssh server v2
ssh server vrf default
ssh server netconf vrf default
!
"#,
    ext = "txt"
)]
pub struct CiscoIosxrZtpTemplate {
    pub hostname: String,
    pub users: Vec<User>,
    pub dns: Dns,
}
