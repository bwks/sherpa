use std::fs;

use anyhow::Result;
use serde_derive::{Deserialize, Serialize};
use toml_edit::{Array, Document, InlineTable, Item, Value};

use crate::data::DeviceModels;
use crate::topology::{Device, Link};
#[derive(Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub name: String,
    pub devices: Vec<Device>,
    pub links: Option<Vec<Link>>,
}

impl Manifest {
    pub fn default() -> Result<Self> {
        let name =
            petname::petname(2, "-").ok_or(anyhow::anyhow!("Failed to generate manifest name"))?;

        let dev01 = Device {
            name: "dev01".to_owned(),
            model: DeviceModels::FedoraLinux,
        };
        let dev02 = Device {
            name: "dev02".to_owned(),
            model: DeviceModels::FedoraLinux,
        };

        let links = vec![Link {
            dev_a: dev01.name.clone(),
            int_a: 1,
            dev_b: dev02.name.clone(),
            int_b: 1,
        }];

        let devices: Vec<Device> = vec![dev01, dev02];

        Ok(Self {
            name,
            devices,
            links: Some(links),
        })
    }
}

impl Manifest {
    pub fn write_file(&self, file_path: &str) -> Result<()> {
        let mut doc = Document::new();

        doc["name"] = Item::Value(Value::from(self.name.to_string()));
        if let Some(table) = doc
            .as_table_mut()
            .get_mut("name")
            .and_then(|v| v.as_value_mut())
        {
            table.decor_mut().set_suffix("\n");
        }

        // Add devices array
        let mut devices_array = Array::new();
        devices_array.set_trailing_comma(true);
        devices_array.set_trailing("\n");
        devices_array.decor_mut().set_suffix("\n");

        for device in &self.devices {
            let mut device_table = InlineTable::new();
            device_table.decor_mut().set_prefix("\n  ");
            device_table.insert("name", Value::from(device.name.as_str()));
            device_table.insert("model", Value::from(device.model.to_string()));
            devices_array.push_formatted(Value::from(device_table));
        }

        doc["devices"] = Item::Value(Value::Array(devices_array));

        // Add links array if present
        if let Some(links) = &self.links {
            let mut link_array = Array::new();
            link_array.set_trailing_comma(true);
            link_array.set_trailing("\n");
            link_array.decor_mut().set_suffix("\n");

            for link in links {
                let mut link_table = InlineTable::new();
                link_table.decor_mut().set_prefix("\n  ");
                link_table.insert("dev_a", Value::from(link.dev_a.as_str()));
                link_table.insert("int_a", Value::from(link.int_a as i64));
                link_table.insert("dev_b", Value::from(link.dev_b.as_str()));
                link_table.insert("int_b", Value::from(link.int_b as i64));
                link_array.push_formatted(Value::from(link_table));
            }
            doc["links"] = Item::Value(Value::Array(link_array));
        }

        fs::write(file_path, doc.to_string())?;
        Ok(())
    }

    pub fn load_file(file_path: &str) -> Result<Manifest> {
        let file_contents = fs::read_to_string(file_path)?;
        let manifest: Manifest = toml::from_str(&file_contents)?;
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::DeviceModels;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_write_file() -> Result<()> {
        // Create temp directory
        let temp_dir = tempdir()?;
        let test_file = temp_dir.path().join("manifest.toml");

        // Set environment variable for test
        std::env::set_var("SHERPA_MANIFEST_FILE", test_file.to_str().unwrap());

        // Create test manifest
        let manifest = Manifest {
            name: "blah".to_string(),
            devices: vec![
                Device {
                    name: "dev01".to_string(),
                    model: DeviceModels::CiscoCat8000v,
                },
                Device {
                    name: "dev02".to_string(),
                    model: DeviceModels::AristaVeos,
                },
            ],
            links: Some(vec![Link {
                dev_a: "dev01".to_string(),
                int_a: 2,
                dev_b: "dev02".to_string(),
                int_b: 1,
            }]),
        };

        // Write manifest
        manifest.write_file(test_file.to_str().unwrap())?;

        // Read and verify contents
        let contents = fs::read_to_string(test_file)?;
        let expected = r#"name = "blah"

devices = [
  { name = "dev01", model = "cisco_cat8000v" },
  { name = "dev02", model = "arista_veos" },
]

links = [
  { dev_a = "dev01", int_a = 2, dev_b = "dev02", int_b = 1 },
]

"#;

        assert_eq!(contents, expected);
        Ok(())
    }
}
