use anyhow::Result;
use std::net::{Ipv4Addr, Ipv6Addr};

use askama::Template;
use serde_derive::Serialize;

use shared::data::{NetworkV4, NetworkV6, SshPublicKey};
use shared::konst::{SHERPA_PASSWORD, SHERPA_USERNAME};

use crate::guest_yaml::{to_cloud_config, to_guest_yaml};

#[derive(Serialize, Debug)]
pub struct CloudbaseInitUser {
    pub name: String,
    pub passwd: String,
    pub groups: Vec<String>,
    pub ssh_authorized_keys: Vec<String>,
}

impl CloudbaseInitUser {
    /// Sherpa's own account, logging in with `ssh_key`: the lab's key, or the server's for labs
    /// created before lab keys.
    #[must_use]
    pub fn sherpa(ssh_key: &SshPublicKey) -> Self {
        Self {
            name: SHERPA_USERNAME.to_owned(),
            passwd: SHERPA_PASSWORD.to_owned(),
            groups: vec!["Administrators".to_string()],
            ssh_authorized_keys: vec![format!("{} {}", ssh_key.algorithm, ssh_key.key)],
        }
    }
}

/// The Windows OpenSSH server config: every account's keys in its own file under
/// `authorized_keys_dir`, named after the account.
#[derive(Template)]
#[template(path = "windows/sshd_config.jinja", ext = "txt")]
pub struct WindowsSshdConfigTemplate {
    /// The directory as sshd names it, such as `__PROGRAMDATA__/ssh/authorized_keys`.
    pub authorized_keys_dir: String,
}

#[derive(Serialize, Debug)]
pub struct CloudbaseWriteFile {
    pub path: String,
    pub content: String,
    pub permissions: String,
}

#[derive(Serialize, Debug)]
pub struct CloudbaseInitConfig {
    pub set_hostname: String,
    pub users: Vec<CloudbaseInitUser>,
    pub write_files: Vec<CloudbaseWriteFile>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub runcmd: Vec<String>,
}

impl CloudbaseInitConfig {
    /// Render the document.
    ///
    /// # Errors
    ///
    /// Fails when the document cannot be serialized.
    pub fn render(&self) -> Result<String> {
        to_cloud_config(self)
    }
}

#[derive(Serialize, Debug)]
pub struct CloudbaseSubnet {
    #[serde(rename = "type")]
    pub type_: String,
    pub address: String,
    pub netmask: String,
    pub gateway: String,
    pub dns_nameservers: Vec<String>,
}

#[derive(Serialize, Debug)]
pub struct CloudbaseNetworkDevice {
    #[serde(rename = "type")]
    pub type_: String,
    pub name: String,
    pub mac_address: String,
    pub subnets: Vec<CloudbaseSubnet>,
}

#[derive(Serialize, Debug)]
pub struct CloudbaseInitNetwork {
    pub version: u8,
    pub config: Vec<CloudbaseNetworkDevice>,
}

impl CloudbaseInitNetwork {
    /// Render the document.
    ///
    /// # Errors
    ///
    /// Fails when the document cannot be serialized.
    pub fn render(&self) -> Result<String> {
        to_guest_yaml(self)
    }

    #[must_use]
    pub fn ztp_interface(
        mgmt_ipv4_address: Ipv4Addr,
        mgmt_mac_address: String,
        mgmt_ipv4: &NetworkV4,
        mgmt_ipv6_address: Option<Ipv6Addr>,
        mgmt_ipv6: Option<&NetworkV6>,
    ) -> Self {
        let mut subnets = vec![CloudbaseSubnet {
            type_: "static".to_string(),
            address: mgmt_ipv4_address.to_string(),
            netmask: mgmt_ipv4.subnet_mask.to_string(),
            gateway: mgmt_ipv4.first.to_string(),
            dns_nameservers: vec![mgmt_ipv4.boot_server.to_string()],
        }];

        if let (Some(ipv6_addr), Some(v6)) = (mgmt_ipv6_address, mgmt_ipv6) {
            subnets.push(CloudbaseSubnet {
                type_: "static6".to_string(),
                address: ipv6_addr.to_string(),
                netmask: format!("{}", v6.prefix_length),
                gateway: v6.first.to_string(),
                dns_nameservers: vec![v6.boot_server.to_string()],
            });
        }

        Self {
            version: 1,
            config: vec![CloudbaseNetworkDevice {
                type_: "physical".to_string(),
                name: "id0".to_string(),
                mac_address: mgmt_mac_address,
                subnets,
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::SshKeyAlgorithms;

    #[test]
    fn the_sherpa_account_takes_the_key_it_is_given() {
        let key = SshPublicKey {
            algorithm: SshKeyAlgorithms::SshRsa,
            key: "AAAAB3NzaC1yc2EAAAADAQABAAABAQDJ".to_owned(),
            comment: None,
        };
        let user = CloudbaseInitUser::sherpa(&key);
        assert_eq!(user.name, SHERPA_USERNAME);
        assert_eq!(
            user.ssh_authorized_keys,
            ["ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQDJ"]
        );
    }
}
