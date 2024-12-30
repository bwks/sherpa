use std::fs;
use std::path::Path;
use std::str::FromStr;

use anyhow::Result;
use ipnetwork::Ipv4Network;
use serde_derive::{Deserialize, Serialize};

use crate::core::konst::{
    QEMU_BIN, SHERPA_BOXES_DIR, SHERPA_CONFIG_DIR, SHERPA_CONFIG_FILE,
    SHERPA_MANAGEMENT_NETWORK_IPV4, SHERPA_PASSWORD, SHERPA_USERNAME,
};
use crate::data::{DeviceModel, VmProviders};
use crate::util::{create_file, expand_path};

#[derive(Serialize, Deserialize, Debug)]
pub struct ZtpServer {
    pub enabled: bool,
    pub username: String,
    pub password: String,
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
    pub ztp_server: ZtpServer,
    pub inventory_management: InventoryManagement,
    pub device_models: Vec<DeviceModel>,
}

impl Default for Config {
    fn default() -> Self {
        let device_models: Vec<DeviceModel> = vec![
            DeviceModel::arista_veos(),
            DeviceModel::arista_ceos(),
            DeviceModel::aruba_aoscx(),
            DeviceModel::cisco_asav(),
            DeviceModel::cisco_cat8000v(),
            DeviceModel::cisco_cat9000v(),
            DeviceModel::cisco_csr1000v(),
            DeviceModel::cisco_iosxrv9000(),
            DeviceModel::cisco_nexus9300v(),
            DeviceModel::cisco_iosv(),
            DeviceModel::cisco_iosvl2(),
            DeviceModel::juniper_vrouter(),
            DeviceModel::juniper_vswitch(),
            DeviceModel::juniper_vevolved(),
            DeviceModel::juniper_vsrxv3(),
            DeviceModel::nokia_srlinux(),
            DeviceModel::cumulus_linux(),
            DeviceModel::centos_linux(),
            DeviceModel::fedora_linux(),
            DeviceModel::redhat_linux(),
            DeviceModel::ubuntu_linux(),
            DeviceModel::opensuse_linux(),
            DeviceModel::suse_linux(),
            DeviceModel::flatcar_linux(),
        ];
        let mgmt_prefix_ipv4 = Ipv4Network::from_str(SHERPA_MANAGEMENT_NETWORK_IPV4)
            .expect("Failed to parse IPv4 network");

        let ztp_server = ZtpServer {
            enabled: false,
            username: SHERPA_USERNAME.to_owned(),
            password: SHERPA_PASSWORD.to_owned(),
        };

        Config {
            name: SHERPA_CONFIG_FILE.to_owned(),
            vm_provider: VmProviders::default(),
            qemu_bin: QEMU_BIN.to_owned(),
            device_models,
            management_prefix_ipv4: mgmt_prefix_ipv4,
            inventory_management: InventoryManagement::default(),
            ztp_server,
        }
    }
}

impl Config {
    pub fn create(&self, path: &str) -> Result<()> {
        let toml_string = toml::to_string_pretty(&self)?;
        create_file(path, toml_string)?;
        Ok(())
    }
    pub fn load(file_path: &str) -> Result<Config> {
        let expanded_path = shellexpand::tilde(file_path);
        let config_path = Path::new(expanded_path.as_ref());

        let contents = fs::read_to_string(config_path)?;
        let config: Config = toml::from_str(&contents)?;
        Ok(config)
    }
}

#[derive(Clone)]
pub struct Sherpa {
    pub config_dir: String,
    pub config_path: String,
    pub boxes_dir: String,
}

impl Default for Sherpa {
    fn default() -> Self {
        let config_dir = expand_path(SHERPA_CONFIG_DIR);
        let boxes_dir = expand_path(&format!("{SHERPA_CONFIG_DIR}/{SHERPA_BOXES_DIR}"));
        let config_path = expand_path(&format!("{SHERPA_CONFIG_DIR}/{SHERPA_CONFIG_FILE}"));
        Self {
            config_dir,
            config_path,
            boxes_dir,
        }
    }
}
