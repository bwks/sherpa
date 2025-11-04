use std::fs;
use std::path::Path;
use std::str::FromStr;

use anyhow::Result;
use ipnetwork::Ipv4Network;
use serde_derive::{Deserialize, Serialize};

use super::container::ContainerImage;
use super::device::DeviceModel;
use super::provider::VmProviders;

use konst::{
    QEMU_BIN, SHERPA_BINS_DIR, SHERPA_BOXES_DIR, SHERPA_CONFIG_DIR, SHERPA_CONFIG_FILE,
    SHERPA_CONTAINERS_DIR, SHERPA_MANAGEMENT_NETWORK_IPV4, SHERPA_PASSWORD, SHERPA_USERNAME,
};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ZtpServer {
    pub enable: bool,
    pub username: Option<String>,
    pub password: Option<String>,
}
impl Default for ZtpServer {
    fn default() -> Self {
        Self {
            enable: false,
            username: Some(SHERPA_USERNAME.to_owned()),
            password: Some(SHERPA_PASSWORD.to_owned()),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct InventoryManagement {
    pub pyats: bool,
    pub ansible: bool,
    pub nornir: bool,
}

impl Default for InventoryManagement {
    fn default() -> Self {
        Self {
            pyats: true,
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
    pub management_prefix_ipv4: Ipv4Network,
    pub boxes_dir: String,
    pub containers_dir: String,
    pub bins_dir: String,
    pub ztp_server: ZtpServer,
    pub inventory_management: InventoryManagement,
    pub device_models: Vec<DeviceModel>,
    pub container_images: Vec<ContainerImage>,
}

#[derive(Clone)]
pub struct Sherpa {
    pub config_dir: String,
    pub config_path: String,
    pub boxes_dir: String,
    pub containers_dir: String,
    pub bins_dir: String,
}
