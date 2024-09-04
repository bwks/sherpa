use std::fs;

use anyhow::Result;
use serde_derive::{Deserialize, Serialize};

use super::{Connection, Device};

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
    pub fn load_file() -> Result<Manifest> {
        let file_contents = fs::read_to_string(MANIFEST_FILENAME)?;
        let manifest: Manifest = toml::from_str(&file_contents)?;
        Ok(manifest)
    }
}
