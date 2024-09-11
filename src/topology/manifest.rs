use std::fs;

use anyhow::Result;
use serde_derive::{Deserialize, Serialize};

use super::{Connection, Device};
use crate::core::konst::MANIFEST_FILE;
use crate::util::generate_id;

#[derive(Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub id: String,
    pub devices: Vec<Device>,
    pub connections: Vec<Connection>,
}

impl Default for Manifest {
    fn default() -> Self {
        let dev1 = Device {
            name: format!("dev1"),
            device_model: crate::model::DeviceModels::NvidiaCumulus,
            id: 1,
        };
        let dev2 = Device {
            name: format!("dev2"),
            device_model: crate::model::DeviceModels::NvidiaCumulus,
            id: 2,
        };

        let connections = vec![Connection {
            device_a: dev1.name.clone(),
            interface_a: 0,
            device_b: dev2.name.clone(),
            interface_b: 0,
        }];

        let devices: Vec<Device> = vec![dev1, dev2];

        Self {
            id: generate_id(),
            devices,
            connections,
        }
    }
}

impl Manifest {
    pub fn write_file(&self) -> Result<()> {
        let toml_string = toml::to_string(&self)?;
        fs::write(MANIFEST_FILE, toml_string)?;

        Ok(())
    }
    pub fn load_file() -> Result<Manifest> {
        let file_contents = fs::read_to_string(MANIFEST_FILE)?;
        let manifest: Manifest = toml::from_str(&file_contents)?;
        Ok(manifest)
    }
}
