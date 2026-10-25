use anyhow::Result;
use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};

use serde_derive::{Deserialize, Serialize};

use shared::data::{NetworkV4, NetworkV6};
use shared::konst::{
    SHERPA_DOMAIN_NAME, SHERPA_PASSWORD, SHERPA_SSH_PUBLIC_KEY_PATH, SHERPA_USERNAME,
};
use shared::util::get_ssh_public_key;

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
    pub fn to_string(&self) -> Result<String> {
        to_guest_yaml(self)
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct CloudInitNetwork {
    pub version: u8,
    pub ethernets: HashMap<String, EthernetInterface>,
}

impl CloudInitNetwork {
    pub fn to_string(&self) -> Result<String> {
        to_guest_yaml(self)
    }
    pub fn ztp_interface(
        mgmt_ipv4_address: Ipv4Addr,
        mgmt_mac_address: String,
        mgmt_ipv4: NetworkV4,
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
    pub ssh_pwauth: bool,
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
    pub runcmd: Option<Vec<String>>,
}
impl CloudInitResolvConf {
    /// Points `/etc/resolv.conf` at the lab DNS, for distros whose network renderer does not
    /// write it.
    pub fn lab_dns(nameserver: Ipv4Addr) -> Self {
        Self {
            domain: SHERPA_DOMAIN_NAME.to_string(),
            searchdomains: vec![SHERPA_DOMAIN_NAME.to_string()],
            nameservers: vec![nameserver],
        }
    }
}

impl CloudInitConfig {
    pub fn to_string(&self) -> Result<String> {
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
    pub sudo: String,
    /// Omitted when empty: cloud-init's schema rejects an empty `groups` list.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
    pub shell: String,
}
impl CloudInitUser {
    pub fn sherpa() -> Result<Self> {
        let ssh_key = get_ssh_public_key(SHERPA_SSH_PUBLIC_KEY_PATH)?;
        Ok(Self {
            name: SHERPA_USERNAME.to_owned(),
            plain_text_passwd: Some(SHERPA_PASSWORD.to_owned()),
            hashed_passwd: None,
            homedir: None,
            lock_passwd: false,
            ssh_authorized_keys: vec![format!("{} {}", ssh_key.algorithm, ssh_key.key.to_owned())],
            sudo: "ALL=(ALL) NOPASSWD:ALL".to_owned(),
            groups: vec!["sudo".to_owned()],
            shell: "/bin/bash".to_owned(),
        })
    }
}
