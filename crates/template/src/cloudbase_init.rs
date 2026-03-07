use anyhow::Result;
use std::net::Ipv4Addr;

use serde_derive::Serialize;

use shared::data::NetworkV4;
use shared::konst::{SHERPA_PASSWORD, SHERPA_SSH_PUBLIC_KEY_PATH, SHERPA_USERNAME};
use shared::util::get_ssh_public_key;

#[derive(Serialize, Debug)]
pub struct CloudbaseInitUser {
    pub name: String,
    pub passwd: String,
    pub groups: Vec<String>,
    pub ssh_authorized_keys: Vec<String>,
}

impl CloudbaseInitUser {
    pub fn sherpa() -> Result<Self> {
        let ssh_key = get_ssh_public_key(SHERPA_SSH_PUBLIC_KEY_PATH)?;
        Ok(Self {
            name: SHERPA_USERNAME.to_owned(),
            passwd: SHERPA_PASSWORD.to_owned(),
            groups: vec!["Administrators".to_string()],
            ssh_authorized_keys: vec![format!("{} {}", ssh_key.algorithm, ssh_key.key)],
        })
    }
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
    pub fn to_string(&self) -> Result<String> {
        let yaml = serde_yaml::to_string(&self)?;
        let mut output = String::from("#cloud-config\n");
        output.push_str(&yaml);
        Ok(output)
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
    pub fn to_string(&self) -> Result<String> {
        Ok(serde_yaml::to_string(&self)?)
    }

    pub fn ztp_interface(
        mgmt_ipv4_address: Ipv4Addr,
        mgmt_mac_address: String,
        mgmt_ipv4: NetworkV4,
    ) -> Self {
        Self {
            version: 1,
            config: vec![CloudbaseNetworkDevice {
                type_: "physical".to_string(),
                name: "id0".to_string(),
                mac_address: mgmt_mac_address,
                subnets: vec![CloudbaseSubnet {
                    type_: "static".to_string(),
                    address: mgmt_ipv4_address.to_string(),
                    netmask: mgmt_ipv4.subnet_mask.to_string(),
                    gateway: mgmt_ipv4.first.to_string(),
                    dns_nameservers: vec![mgmt_ipv4.boot_server.to_string()],
                }],
            }],
        }
    }
}
