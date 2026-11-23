use std::net::Ipv4Addr;

use askama::Template;

/// Fully resolved values for one SSH host block.
#[derive(Debug, Clone)]
pub struct SshHostConfig {
    pub aliases: String,
    pub hostname: Ipv4Addr,
    pub port: u16,
    pub username: String,
    /// The lab `known_hosts` file holding this node's host key, which is then checked
    /// strictly. `None` when no key was recorded, and the key is not checked.
    pub known_hosts_file: Option<String>,
}

#[derive(Template)]
#[template(path = "ssh/ssh_config.jinja", ext = "txt")]
pub struct SshConfigTemplate {
    pub hosts: Vec<SshHostConfig>,
}

/// One node's host key in a lab `known_hosts` file, named by the node's address: nodes take
/// SSH on port 22.
#[derive(Debug, Clone)]
pub struct KnownHostsEntry {
    pub hostname: Ipv4Addr,
    /// The key in OpenSSH public key form (`ssh-ed25519 AAAA...`).
    pub key: String,
}

#[derive(Template)]
#[template(path = "ssh/known_hosts.jinja", ext = "txt")]
pub struct KnownHostsTemplate {
    pub entries: Vec<KnownHostsEntry>,
}
