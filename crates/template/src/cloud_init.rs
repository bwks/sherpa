use anyhow::Result;
use std::collections::HashMap;
use std::net::Ipv4Addr;

use serde_derive::{Deserialize, Serialize};

use data::NetworkV4;
use konst::{SHERPA_CONFIG_DIR, SHERPA_PASSWORD, SHERPA_SSH_PUBLIC_KEY_FILE, SHERPA_USERNAME};
use util::get_ssh_public_key;

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct MetaDataConfig {
    #[serde(rename = "instance-id")]
    pub instance_id: String,
    #[serde(rename = "local-hostname")]
    pub local_hostname: String,
}
impl MetaDataConfig {
    pub fn to_string(&self) -> Result<String> {
        Ok(serde_yaml::to_string(&self)?)
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct CloudInitNetwork {
    pub version: u8,
    pub ethernets: HashMap<String, EthernetInterface>,
}

impl CloudInitNetwork {
    pub fn to_string(&self) -> Result<String> {
        Ok(serde_yaml::to_string(&self)?)
    }
    pub fn ztp_interface(
        mgmt_ipv4_address: Ipv4Addr,
        mgmt_mac_address: String,
        mgmt_ipv4: NetworkV4,
    ) -> Self {
        let mut ethernets = HashMap::new();

        ethernets.insert(
            "id0".to_string(),
            EthernetInterface {
                match_config: MatchConfig {
                    macaddress: mgmt_mac_address,
                },
                addresses: vec![format!("{}/{}", mgmt_ipv4_address, mgmt_ipv4.prefix_length)],
                routes: vec![Route {
                    to: "0.0.0.0/0".to_string(),
                    via: mgmt_ipv4.first.to_string(),
                }],
                nameservers: Nameservers {
                    addresses: vec![mgmt_ipv4.boot_server.to_string()],
                },
            },
        );
        Self {
            version: 2,
            ethernets: ethernets,
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
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct CloudInitConfig {
    pub hostname: String,
    pub fqdn: String,
    pub manage_etc_hosts: bool,
    pub ssh_pwauth: bool,
    pub users: Vec<CloudInitUser>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packages: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runcmd: Option<Vec<String>>,
}
impl CloudInitConfig {
    pub fn to_string(&self) -> Result<String> {
        // First serialize to regular YAML
        let yaml = serde_yaml::to_string(&self)?;

        // Prepend the #cloud-config comment
        let mut output = String::from("#cloud-config\n");
        output.push_str(&yaml);

        Ok(output)
    }
}
#[derive(Serialize, Deserialize, Debug)]
pub struct CloudInitUser {
    pub name: String,
    pub plain_text_passwd: String,
    pub lock_passwd: bool,
    pub ssh_authorized_keys: Vec<String>,
    pub sudo: String,
    pub groups: Vec<String>,
    pub shell: String,
}
impl CloudInitUser {
    pub fn sherpa() -> Result<Self> {
        let ssh_key =
            get_ssh_public_key(&format!("{SHERPA_CONFIG_DIR}/{SHERPA_SSH_PUBLIC_KEY_FILE}"))?;
        Ok(Self {
            name: SHERPA_USERNAME.to_owned(),
            plain_text_passwd: SHERPA_PASSWORD.to_owned(),
            lock_passwd: false,
            ssh_authorized_keys: vec![format!("{} {}", ssh_key.algorithm, ssh_key.key.to_owned())],
            sudo: "ALL=(ALL) NOPASSWD:ALL".to_owned(),
            groups: vec!["sudo".to_owned()],
            shell: "/bin/bash".to_owned(),
        })
    }
}
