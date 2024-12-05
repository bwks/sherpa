use std::fs;

use anyhow::Result;
use serde_derive::{Deserialize, Serialize};
use toml_edit::{Array, Document, InlineTable, Item, Value};

use crate::data::DeviceModels;
use crate::topology::{Connection, Device};
#[derive(Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub name: String,
    pub devices: Vec<Device>,
    pub connections: Option<Vec<Connection>>,
}

impl Manifest {
    pub fn default() -> Result<Self> {
        let name =
            petname::petname(2, "-").ok_or(anyhow::anyhow!("Failed to generate manifest name"))?;

        let dev01 = Device {
            name: "dev01".to_owned(),
            device_model: DeviceModels::FedoraLinux,
        };
        let dev02 = Device {
            name: "dev02".to_owned(),
            device_model: DeviceModels::FedoraLinux,
        };

        let connections = vec![Connection {
            device_a: dev01.name.clone(),
            interface_a: 1,
            device_b: dev02.name.clone(),
            interface_b: 1,
        }];

        let devices: Vec<Device> = vec![dev01, dev02];

        Ok(Self {
            name,
            devices,
            connections: Some(connections),
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
            device_table.insert("device_model", Value::from(device.device_model.to_string()));
            devices_array.push_formatted(Value::from(device_table));
        }

        doc["devices"] = Item::Value(Value::Array(devices_array));

        // Add connections array if present
        if let Some(connections) = &self.connections {
            let mut conn_array = Array::new();
            conn_array.set_trailing_comma(true);
            conn_array.set_trailing("\n");
            conn_array.decor_mut().set_suffix("\n");

            for conn in connections {
                let mut conn_table = InlineTable::new();
                conn_table.decor_mut().set_prefix("\n  ");
                conn_table.insert("device_a", Value::from(conn.device_a.as_str()));
                conn_table.insert("interface_a", Value::from(conn.interface_a as i64));
                conn_table.insert("device_b", Value::from(conn.device_b.as_str()));
                conn_table.insert("interface_b", Value::from(conn.interface_b as i64));
                conn_array.push_formatted(Value::from(conn_table));
            }
            doc["connections"] = Item::Value(Value::Array(conn_array));
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
                    device_model: DeviceModels::CiscoCat8000v,
                },
                Device {
                    name: "dev02".to_string(),
                    device_model: DeviceModels::AristaVeos,
                },
            ],
            connections: Some(vec![Connection {
                device_a: "dev01".to_string(),
                interface_a: 2,
                device_b: "dev02".to_string(),
                interface_b: 1,
            }]),
        };

        // Write manifest
        manifest.write_file(test_file.to_str().unwrap())?;

        // Read and verify contents
        let contents = fs::read_to_string(test_file)?;
        let expected = r#"name = "blah"

devices = [
  { name = "dev01", device_model = "cisco_cat8000v" },
  { name = "dev02", device_model = "arista_veos" },
]

connections = [
  { device_a = "dev01", interface_a = 2, device_b = "dev02", interface_b = 1 },
]

"#;

        assert_eq!(contents, expected);
        Ok(())
    }
}
