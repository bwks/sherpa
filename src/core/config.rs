use std::fs;

use anyhow::Result;
use serde_derive::Serialize;

use super::konst::CONFIG_FILENAME;

#[derive(Serialize)]
pub struct Config {
    pub name: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            name: CONFIG_FILENAME.to_owned(),
        }
    }
}

impl Config {
    pub fn write_file(&self) -> Result<()> {
        let toml_string = toml::to_string(&self)?;
        fs::write(CONFIG_FILENAME, toml_string)?;

        Ok(())
    }
}
