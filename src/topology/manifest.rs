use std::fs;

use anyhow::Result;
use serde_derive::{Deserialize, Serialize};

use super::{Connection, Device};
use crate::model::{DeviceModel, DeviceModels};

use crate::core::konst::MANIFEST_FILENAME;

#[derive(Default, Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub devices: Vec<Device>,
    pub connections: Vec<Connection>,
}

impl Manifest {
    pub fn write_file(&self) -> Result<()> {
        let toml_string = toml::to_string(&self)?;
        fs::write(MANIFEST_FILENAME, toml_string)?;

        Ok(())
    }
    pub fn load_file(&self) -> Result<()> {
        let file_contents = fs::read_to_string(MANIFEST_FILENAME)?;
        let mut manifest: Manifest = toml::from_str(&file_contents)?;

        // for device in &mut manifest.devices {
        //     match DeviceModels::from_str(&device.device_type) {
        //         DeviceModels::Cat9kv => device.device_model = Some(DeviceModel::cisco_cat9kv()),
        //         _ => println!("Device model not supported yet"),
        //     }
        // }

        println!("{:#?}", manifest);

        Ok(())
    }
}
