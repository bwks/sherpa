use std::net::Ipv4Addr;

use askama::Template;

use crate::model::User;

#[derive(Template)]
#[template(
    source = r#"!
console serial
!
interface Management0/0
 nameif management
 management-only
 security-level 0
 ip address dhcp
 no shutdown
!
hostname {{ hostname }}
!
username enable_1 privilege 15
enable password {{ crate::core::konst::SHERPA_PASSWORD }}
{%- for user in users %}
username {{ user.username }} privilege 15
{%-   if let Some(password) = user.password %} 
username {{ user.username }} password {{ password }}
{%-   endif %}
{%- endfor %}
!
aaa authentication ssh console LOCAL
aaa authentication http console LOCAL
aaa authorization exec LOCAL auto-enable
no ssh stack ciscossh
crypto key generate rsa modulus 2048 noconfirm
ssh 0.0.0.0 0.0.0.0 management
http server enable
http 0.0.0.0 0.0.0.0 management
domain-name {{ crate::core::konst::SHERPA_DOMAIN_NAME }}
!
{%- for user in users %}
{%-   if let Some(password) = user.password %}
username {{ user.username }} password {{ password }} 
{%-   endif %}
username {{ user.username }} attributes
{%-   if user.sudo %}
  service-type admin
{%-   endif %}
  ssh authentication publickey {{ user.ssh_public_key.key }} hashed
{%- endfor %}
!
names
dns domain-lookup management
dns name-server {{ name_server }}
!
"#,
    ext = "txt"
)]
pub struct CiscoAsavZtpTemplate {
    pub hostname: String,
    pub users: Vec<User>,
    pub name_server: Ipv4Addr,
}
