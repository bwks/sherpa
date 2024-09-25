use std::fs;
use std::path::Path;

use anyhow::Result;
use serde_derive::{Deserialize, Serialize};

use super::konst::{BOXES_DIR, CONFIG_DIR, CONFIG_FILE, QEMU_BIN};
use crate::model::DeviceModel;
use crate::model::VmProviders;
use crate::util::{create_file, expand_path};

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub name: String,
    pub vm_provider: VmProviders,
    pub qemu_bin: String,
    pub device_models: Vec<DeviceModel>,
}

impl Default for Config {
    fn default() -> Self {
        let device_models: Vec<DeviceModel> = vec![
            DeviceModel::arista_veos(),
            DeviceModel::cisco_asav(),
            DeviceModel::cisco_cat8000v(),
            DeviceModel::cisco_cat9000v(),
            DeviceModel::cisco_csr1000v(),
            DeviceModel::cisco_iosxrv9000(),
            DeviceModel::cisco_nexus9300v(),
            DeviceModel::cisco_iosv(),
            DeviceModel::cisco_iosvl2(),
            DeviceModel::juniper_vjunos_router(),
            DeviceModel::juniper_vjunos_switch(),
            DeviceModel::nokia_vsr(),
            DeviceModel::cumulus_linux(),
            DeviceModel::centos_linux(),
            DeviceModel::fedora_linux(),
            DeviceModel::redhat_linux(),
            DeviceModel::ubuntu_linux(),
            DeviceModel::opensuse_linux(),
            DeviceModel::suse_linux(),
        ];
        Config {
            name: CONFIG_FILE.to_owned(),
            vm_provider: VmProviders::default(),
            qemu_bin: QEMU_BIN.to_owned(),
            device_models,
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
        let config_dir = expand_path(CONFIG_DIR);
        let boxes_dir = expand_path(&format!("{config_dir}/{BOXES_DIR}"));
        let config_path = expand_path(&format!("{CONFIG_DIR}/{CONFIG_FILE}"));
        Self {
            config_dir,
            config_path,
            boxes_dir,
        }
    }
}
