use std::fs;

use anyhow::Result;
use serde_derive::Serialize;

use super::konst::{CONFIG_FILENAME, QEMU_BIN};
use crate::model::DeviceModel;
use crate::model::VmProviders;
#[derive(Serialize)]
pub struct Config {
    pub name: String,
    vm_provider: VmProviders,
    qemu_bin: String,
    device_models: Vec<DeviceModel>,
}

impl Default for Config {
    fn default() -> Self {
        let mut device_models: Vec<DeviceModel> = vec![];

        device_models.push(DeviceModel::arista_veos());
        device_models.push(DeviceModel::cisco_csr1000v());
        device_models.push(DeviceModel::cisco_cat9000v());
        device_models.push(DeviceModel::cisco_cat8000v());
        device_models.push(DeviceModel::cisco_iosxrv9000());
        device_models.push(DeviceModel::cisco_nexus9300v());
        device_models.push(DeviceModel::cisco_iosv());
        device_models.push(DeviceModel::cisco_iosvl2());
        device_models.push(DeviceModel::nokia_sros());
        device_models.push(DeviceModel::nvidia_cumulus());

        Config {
            name: CONFIG_FILENAME.to_owned(),
            vm_provider: VmProviders::default(),
            qemu_bin: QEMU_BIN.to_owned(),
            device_models,
        }
    }
}

impl Config {
    pub fn write_file(&self) -> Result<()> {
        let toml_string = toml::to_string_pretty(&self)?;
        fs::write(CONFIG_FILENAME, toml_string)?;
        Ok(())
    }
}
