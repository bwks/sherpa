use askama::Template;

use crate::core::konst::ARUBA_ZTP_SCRIPT;
use crate::core::konst::SHERPA_PASSWORD;
use crate::data::{Dns, User};

#[allow(dead_code)]
pub fn aruba_aoscx_ztp_script() -> String {
    format!(
        r#"!
start-shell
sudo mkdir /mnt/config/
sudo mount /dev/sdb /mnt/config/
/bin/sh /mnt/config/{ARUBA_ZTP_SCRIPT}
    "#,
    )
}

#[derive(Template)]
#[template(
    source = r#"!
hostname {{ hostname }}
domain-name {{ dns.domain }}
{%- for server in dns.name_servers %}
ip dns server-address {{ server.ipv4_address }}
{%- endfor %}
user admin group administrators password plaintext {{ SHERPA_PASSWORD }}
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
    pub dns: Dns,
}

#[derive(Template)]
#[template(
    source = r#"/usr/bin/vtysh \
-c 'configure' \
-c '!' \
-c 'hostname {{ hostname }}' \
-c 'domain-name {{ dns.domain }}' \
{%- for server in dns.name_servers %}
-c 'ip dns server-address {{ server.ipv4_address }}' \
{%- endfor %}
-c 'user admin group administrators password plaintext {{ SHERPA_PASSWORD }}' \
{%- for user in users %}
-c 'user {{ user.username }} {% if user.sudo %} group administrators{% endif %}{% if let Some(password) = user.password %} password plaintext {{ password }}{% endif %}' \
{%- endfor %}
-c '!' \
-c 'ntp server pool.ntp.org minpoll 4 maxpoll 4 iburst' \
-c 'ntp enable' \
-c '!' \
{%- for user in users %}
-c 'user {{ user.username }} authorized-key {{ user.ssh_public_key.algorithm }} {{ user.ssh_public_key.key }}' \
{%- endfor %}
-c '!' \
-c 'ssh server vrf mgmt' \
-c 'vlan 1' \
-c 'interface mgmt' \
-c '    no shutdown' \
-c '    ip dhcp' \
-c '!' \
-c 'https-server vrf mgmt'
"#,
    ext = "txt"
)]
pub struct ArubaAoscxShTemplate {
    pub hostname: String,
    pub users: Vec<User>,
    pub dns: Dns,
}
