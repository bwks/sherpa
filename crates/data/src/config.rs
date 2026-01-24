use ipnet::Ipv4Net;
use serde_derive::{Deserialize, Serialize};

use super::container::ContainerImage;
use super::node::NodeVariant;
use super::provider::VmProviders;

use konst::{SHERPA_PASSWORD, SHERPA_USERNAME};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ZtpServer {
    pub enable: bool,
    pub username: Option<String>,
    pub password: Option<String>,
}
impl Default for ZtpServer {
    fn default() -> Self {
        Self {
            enable: true,
            username: Some(SHERPA_USERNAME.to_owned()),
            password: Some(SHERPA_PASSWORD.to_owned()),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InventoryManagement {
    pub ansible: bool,
    pub pyats: bool,
    pub nornir: bool,
}

impl Default for InventoryManagement {
    fn default() -> Self {
        Self {
            pyats: false,
            ansible: false,
            nornir: false,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub name: String,
    pub vm_provider: VmProviders,
    pub qemu_bin: String,
    pub management_prefix_ipv4: Ipv4Net,
    pub images_dir: String,
    pub containers_dir: String,
    pub bins_dir: String,
    pub ztp_server: ZtpServer,
    pub inventory_management: InventoryManagement,
    pub device_models: Vec<NodeVariant>,
    pub container_images: Vec<ContainerImage>,
}

#[derive(Clone, Debug)]
pub struct Sherpa {
    pub base_dir: String,
    pub config_dir: String,
    pub config_file_path: String,
    pub ssh_dir: String,
    pub images_dir: String,
    pub containers_dir: String,
    pub bins_dir: String,
}
