use askama::Template;

use crate::data::{Dns, User};

#[derive(Template)]
#[template(
    source = r#"#!/bin/bash

# CUMULUS-AUTOPROVISIONING

function error() {
  echo -e "\e[0;33mERROR: The ZTP script failed while running the command $BASH_COMMAND at line $BASH_LINENO.\e[0m" >&2
  exit 1
}

# Log all output from this script
exec >> /var/log/autoprovision 2>&1
date "+%FT%T ztp starting script $0"

trap error ERR

#Configs
nv set system hostname {{ hostname }}
nv set service dns default search {{ dns.domain }}
{% for server in dns.name_servers %}
nv set service dns default server {{ server.ipv4_address }}
{% endfor %}
{%- for user in users %}
nv set system aaa user {{ user.username }}
{%-   if let Some(password) = user.password %}
nv set system aaa user {{ user.username }} password '{{ password }}'
{%- endif %}
nv set system aaa user {{ user.username }} ssh authorized-key {{ user.username }}-ssh-key key {{ user.ssh_public_key.key }}
nv set system aaa user {{ user.username }} ssh authorized-key {{ user.username }}-ssh-key type {{ user.ssh_public_key.algorithm }}
{%-   if user.sudo %}
nv set system aaa user {{ user.username }} role system-admin
{%   endif %}
{%- endfor %}

nv config apply --assume-yes --message "ZTP config"

exit 0
"#,
    ext = "txt"
)]
pub struct CumulusLinuxZtpTemplate {
    pub hostname: String,
    pub users: Vec<User>,
    pub dns: Dns,
}
