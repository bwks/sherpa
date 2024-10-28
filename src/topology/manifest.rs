use std::fs;

use anyhow::Result;
use serde_derive::{Deserialize, Serialize};

use super::{Connection, Device};
use crate::core::konst::MANIFEST_FILE;

#[derive(Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub devices: Vec<Device>,
    pub connections: Option<Vec<Connection>>,
}

impl Default for Manifest {
    fn default() -> Self {
        let dev1 = Device {
            name: "dev1".to_owned(),
            device_model: crate::model::DeviceModels::FedoraLinux,
            id: 1,
        };
        let dev2 = Device {
            name: "dev2".to_owned(),
            device_model: crate::model::DeviceModels::FedoraLinux,
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
            devices,
            connections: Some(connections),
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
