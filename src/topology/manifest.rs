use std::fs;

use anyhow::Result;
use serde_derive::{Deserialize, Serialize};

use crate::core::konst::SHERPA_MANIFEST_FILE;
use crate::data::DeviceModels;
use crate::topology::{Connection, Device};

#[derive(Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub devices: Vec<Device>,
    pub connections: Option<Vec<Connection>>,
}

impl Default for Manifest {
    fn default() -> Self {
        let dev01 = Device {
            name: "dev01".to_owned(),
            device_model: DeviceModels::FedoraLinux,
            id: 1,
        };
        let dev02 = Device {
            name: "dev02".to_owned(),
            device_model: DeviceModels::FedoraLinux,
            id: 2,
        };

        let connections = vec![Connection {
            device_a: dev01.name.clone(),
            interface_a: 0,
            device_b: dev02.name.clone(),
            interface_b: 0,
        }];

        let devices: Vec<Device> = vec![dev01, dev02];

        Self {
            devices,
            connections: Some(connections),
        }
    }
}

impl Manifest {
    pub fn write_file(&self) -> Result<()> {
        let toml_string = toml::to_string(&self)?;
        fs::write(SHERPA_MANIFEST_FILE, toml_string)?;

        Ok(())
    }
    pub fn load_file() -> Result<Manifest> {
        let file_contents = fs::read_to_string(SHERPA_MANIFEST_FILE)?;
        let manifest: Manifest = toml::from_str(&file_contents)?;
        Ok(manifest)
    }
}
