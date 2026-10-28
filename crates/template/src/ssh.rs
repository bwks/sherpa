use std::net::Ipv4Addr;

use askama::Template;

use shared::konst::SSH_PORT;

/// Fully resolved values for one SSH host block.
#[derive(Debug, Clone)]
pub struct SshHostConfig {
    pub aliases: String,
    pub hostname: Ipv4Addr,
    pub port: u16,
    pub username: String,
    pub identity_file: Option<String>,
    /// The lab `known_hosts` file holding this node's host key, which is then checked
    /// strictly. `None` when no key was recorded, and the key is not checked.
    pub known_hosts_file: Option<String>,
}

#[derive(Template)]
#[template(path = "ssh/ssh_config.jinja", ext = "txt")]
pub struct SshConfigTemplate {
    pub hosts: Vec<SshHostConfig>,
    pub proxy_jump: String,
}

/// One node's host key in a lab `known_hosts` file.
#[derive(Debug, Clone)]
pub struct KnownHostsEntry {
    pub hostname: Ipv4Addr,
    pub port: u16,
    /// The key in OpenSSH public key form (`ssh-ed25519 AAAA...`).
    pub key: String,
}

#[derive(Template)]
#[template(path = "ssh/known_hosts.jinja", ext = "txt")]
pub struct KnownHostsTemplate {
    pub entries: Vec<KnownHostsEntry>,
}

impl KnownHostsEntry {
    /// The name OpenSSH looks the node up by: the address on port 22, else `[address]:port`.
    pub fn name(&self) -> String {
        if self.port == SSH_PORT {
            self.hostname.to_string()
        } else {
            format!("[{}]:{}", self.hostname, self.port)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_precomputed_usernames_and_optional_identity_files() {
        let output = SshConfigTemplate {
            hosts: vec![
                SshHostConfig {
                    aliases: "192.0.2.10 vm.abcd1234 vm.abcd1234.sherpa.lab.local".to_owned(),
                    hostname: "192.0.2.10".parse().unwrap(),
                    port: 22,
                    username: "alice".to_owned(),
                    identity_file: None,
                    known_hosts_file: None,
                },
                SshHostConfig {
                    aliases: "192.0.2.11 router.abcd1234 router.abcd1234.sherpa.lab.local"
                        .to_owned(),
                    hostname: "192.0.2.11".parse().unwrap(),
                    port: 830,
                    username: "provided-user".to_owned(),
                    identity_file: Some("provided-key".to_owned()),
                    known_hosts_file: None,
                },
            ],
            proxy_jump: "owner@192.0.2.1".to_owned(),
        }
        .render()
        .unwrap();
        let (personal, explicit_identity) = output.split_once("Host 192.0.2.11").unwrap();
        assert!(personal.contains("User alice"));
        assert!(personal.contains("vm.abcd1234.sherpa.lab.local"));
        assert!(personal.contains("ProxyJump owner@192.0.2.1"));
        assert!(!personal.contains("IdentityFile"));
        assert!(!personal.contains("IdentitiesOnly"));
        assert!(explicit_identity.contains("User provided-user"));
        assert!(explicit_identity.contains("Port 830"));
        assert!(explicit_identity.contains("IdentityFile provided-key"));
        assert!(explicit_identity.contains("IdentitiesOnly yes"));
    }
}
