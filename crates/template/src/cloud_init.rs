use anyhow::Result;
use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};

use serde::Serializer;
use serde_derive::{Deserialize, Serialize};

use shared::data::{NetworkV4, NetworkV6, SshPublicKey};
use shared::konst::{
    NODE_CLOUD_INIT_SUDO_RULE, SHERPA_DOMAIN_NAME, SHERPA_PASSWORD, SHERPA_USERNAME,
};

use crate::guest_yaml::{to_cloud_config, to_guest_yaml};

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct MetaDataConfig {
    #[serde(rename = "instance-id")]
    pub instance_id: String,
    #[serde(rename = "local-hostname")]
    pub local_hostname: String,
    #[serde(rename = "public-keys", skip_serializing_if = "Vec::is_empty")]
    pub public_keys: Vec<String>,
}
impl MetaDataConfig {
    /// Render the document.
    ///
    /// # Errors
    ///
    /// Fails when the document cannot be serialized.
    pub fn render(&self) -> Result<String> {
        to_guest_yaml(self)
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct CloudInitNetwork {
    pub version: u8,
    pub ethernets: HashMap<String, EthernetInterface>,
}

impl CloudInitNetwork {
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
        let mut addresses = vec![format!("{}/{}", mgmt_ipv4_address, mgmt_ipv4.prefix_length)];
        let mut routes = vec![Route {
            to: "0.0.0.0/0".to_string(),
            via: mgmt_ipv4.first.to_string(),
        }];
        let mut dns_addresses = vec![mgmt_ipv4.boot_server.to_string()];

        if let (Some(ipv6_addr), Some(v6)) = (mgmt_ipv6_address, mgmt_ipv6) {
            addresses.push(format!("{}/{}", ipv6_addr, v6.prefix_length));
            routes.push(Route {
                to: "::/0".to_string(),
                via: v6.first.to_string(),
            });
            dns_addresses.push(v6.boot_server.to_string());
        }

        let mut ethernets = HashMap::new();
        ethernets.insert(
            "id0".to_string(),
            EthernetInterface {
                match_config: MatchConfig {
                    macaddress: mgmt_mac_address,
                },
                addresses,
                routes,
                nameservers: Nameservers {
                    search: vec![SHERPA_DOMAIN_NAME.to_string()],
                    addresses: dns_addresses,
                },
            },
        );
        Self {
            version: 2,
            ethernets,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct EthernetInterface {
    #[serde(rename = "match")]
    pub match_config: MatchConfig,
    pub addresses: Vec<String>,
    pub routes: Vec<Route>,
    pub nameservers: Nameservers,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MatchConfig {
    pub macaddress: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Route {
    pub to: String,
    pub via: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Nameservers {
    pub addresses: Vec<String>,
    pub search: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CloudInitResolvConf {
    pub domain: String,
    pub searchdomains: Vec<String>,
    pub nameservers: Vec<Ipv4Addr>,
}

/// CA certificates cloud-init adds to the system trust store (`ca_certs` module).
#[derive(Serialize, Deserialize, Debug)]
pub struct CloudInitCaCerts {
    /// PEM certificates.
    pub trusted: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CloudInitWriteFile {
    pub path: String,
    pub content: String,
    pub permissions: String,
    /// `None` uses the distro's default owner, `root:root`, or `root:wheel` on BSD, which has
    /// no `root` group.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct CloudInitConfig {
    pub hostname: String,
    pub fqdn: String,
    pub manage_etc_hosts: bool,
    /// Omitted where the image owns the SSH server's settings, so cloud-init leaves them alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_pwauth: Option<bool>,
    pub users: Vec<CloudInitUser>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manage_resolv_conf: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolv_conf: Option<CloudInitResolvConf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packages: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_files: Option<Vec<CloudInitWriteFile>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ca_certs: Option<CloudInitCaCerts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runcmd: Option<Vec<String>>,
}
impl CloudInitResolvConf {
    /// Points `/etc/resolv.conf` at the lab DNS, for distros whose network renderer does not
    /// write it.
    #[must_use]
    pub fn lab_dns(nameserver: Ipv4Addr) -> Self {
        Self {
            domain: SHERPA_DOMAIN_NAME.to_string(),
            searchdomains: vec![SHERPA_DOMAIN_NAME.to_string()],
            nameservers: vec![nameserver],
        }
    }
}

impl CloudInitConfig {
    /// Render the document.
    ///
    /// # Errors
    ///
    /// Fails when the document cannot be serialized.
    pub fn render(&self) -> Result<String> {
        to_cloud_config(self)
    }
}
#[derive(Serialize, Deserialize, Debug)]
pub struct CloudInitUser {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plain_text_passwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hashed_passwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homedir: Option<String>,
    pub lock_passwd: bool,
    pub ssh_authorized_keys: Vec<String>,
    /// Omitted where sudo is not the native admin tool.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sudo: Option<String>,
    /// Written as a comma-separated string, which every distro accepts: cloud-init's BSD
    /// `add_user` drops a list. Omitted when empty, which cloud-init's schema rejects.
    #[serde(
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_comma_separated"
    )]
    pub groups: Vec<String>,
    pub shell: String,
}
impl CloudInitUser {
    /// Sherpa's own account, logging in with `ssh_key`: the lab's key, or the server's for labs
    /// created before lab keys.
    #[must_use]
    pub fn sherpa(ssh_key: &SshPublicKey) -> Self {
        Self {
            name: SHERPA_USERNAME.to_owned(),
            plain_text_passwd: Some(SHERPA_PASSWORD.to_owned()),
            hashed_passwd: None,
            homedir: None,
            lock_passwd: false,
            ssh_authorized_keys: vec![format!("{} {}", ssh_key.algorithm, ssh_key.key)],
            sudo: Some(NODE_CLOUD_INIT_SUDO_RULE.to_owned()),
            groups: vec!["sudo".to_owned()],
            shell: "/bin/bash".to_owned(),
        }
    }
}

fn serialize_comma_separated<S>(values: &[String], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&values.join(","))
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
        let user = CloudInitUser::sherpa(&key);
        assert_eq!(user.name, SHERPA_USERNAME);
        assert_eq!(
            user.ssh_authorized_keys,
            ["ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQDJ"]
        );
    }
}
