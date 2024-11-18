use askama::Template;

use crate::core::konst::ARUBA_ZTP_CONFIG;
use crate::data::{Dns, User};

#[allow(dead_code)]
pub fn aruba_aoscx_ztp_config() -> String {
    format!(
        r#"!
# usb
# usb mount
# copy usb:/{ARUBA_ZTP_CONFIG} running-config
# write memory
start-shell
mount /dev/sdb /mnt/external-storage/
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
    pub dns: Dns,
}
