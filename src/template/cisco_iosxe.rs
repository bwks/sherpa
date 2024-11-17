use askama::Template;

use crate::data::Dns;
use crate::model::User;

#[derive(Template)]
#[template(
    source = r#"!
hostname {{ hostname }}
ip domain name {{ dns.domain }}
{%- for server in dns.name_servers %}
ip name-server {{ server.ipv4_address }}
{%- endfor %}
crypto key generate rsa modulus 2048
ip ssh version 2
!
aaa new-model
aaa authentication login default local
aaa authorization exec default local
!
{%- for user in users %}
username {{ user.username }} privilege 15{% if let Some(password) = user.password %} secret {{ password }}{% endif %}
{%- endfor %}
!
ip ssh pubkey-chain
{%- for user in users %}
  username {{ user.username }}
   key-hash {{ user.ssh_public_key.algorithm }} {{ user.ssh_public_key.key }}
{%- endfor %}
!
!
interface {{ mgmt_interface }}
 ip address dhcp
 negotiation auto
 no shutdown
 exit
!
line con 0
 logging synchronous
 stopbits 1
 exit
!
line vty 0 4
 logging synchronous
 transport input ssh
 exit
!
exit
"#,
    ext = "txt"
)]
pub struct CiscoIosXeZtpTemplate {
    pub hostname: String,
    pub users: Vec<User>,
    pub mgmt_interface: String,
    pub dns: Dns,
}
