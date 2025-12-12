use std::fs;

use anyhow::Result;
use serde_derive::{Deserialize, Serialize};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Value};

use super::node::Node;
use super::link::Link2;
use data::{DeviceModels, ZtpServer};

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct Manifest {
    pub name: String,
    pub nodes: Vec<Node>,
    pub links: Option<Vec<Link2>>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ztp_server: Option<ZtpServer>,
}

impl Manifest {
    pub fn example() -> Result<Self> {
        let name =
            petname::petname(2, "-").ok_or(anyhow::anyhow!("Failed to generate manifest name"))?;

        let dev01 = Node {
            name: "dev01".to_owned(),
            model: DeviceModels::FedoraLinux,
            ..Default::default()
        };
        let dev02 = Node {
            name: "dev02".to_owned(),
            model: DeviceModels::FedoraLinux,
            ..Default::default()
        };

        let links = vec![Link2 {
            src: format!("{}::{}", dev01.name.clone(), "eth1"),
            dst: format!("{}::{}", dev02.name.clone(), "eth1"),
        }];

        let nodes: Vec<Node> = vec![dev01, dev02];

        Ok(Self {
            name,
            nodes,
            links: Some(links),
            ..Default::default()
        })
    }
}

impl Manifest {
    pub fn write_file(&self, file_path: &str) -> Result<()> {
        let mut doc = DocumentMut::new();

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

        for device in &self.nodes {
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
                link_table.insert("src", Value::from(link.src.as_str()));
                link_table.insert("src", Value::from(link.dst.as_str()));
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

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use crate::data::DeviceModels;
//     use std::fs;
//     use tempfile::tempdir;

//     #[test]
//     fn test_write_file() -> Result<()> {
//         // Create temp directory
//         let temp_dir = tempdir()?;
//         let test_file = temp_dir.path().join("manifest.toml");

//         // Set environment variable for test
//         std::env::set_var("SHERPA_MANIFEST_FILE", test_file.to_str().unwrap());

//         // Create test manifest
//         let manifest = Manifest {
//             name: "blah".to_string(),
//             devices: vec![
//                 Device {
//                     name: "dev01".to_string(),
//                     model: DeviceModels::CiscoCat8000v,
//                     ..Default::default()
//                 },
//                 Device {
//                     name: "dev02".to_string(),
//                     model: DeviceModels::AristaVeos,
//                     ..Default::default()
//                 },
//             ],
//             links: Some(vec![Link {
//                 dev_a: "dev01".to_string(),
//                 int_a: 2,
//                 dev_b: "dev02".to_string(),
//                 int_b: 1,
//             }]),
//             ..Default::default()
//         };

//         // Write manifest
//         manifest.write_file(test_file.to_str().unwrap())?;

//         // Read and verify contents
//         let contents = fs::read_to_string(test_file)?;
//         let expected = r#"name = "blah"

// devices = [
//   { name = "dev01", model = "cisco_cat8000v" },
//   { name = "dev02", model = "arista_veos" },
// ]

// links = [
//   { dev_a = "dev01", int_a = 2, dev_b = "dev02", int_b = 1 },
// ]

// "#;

//         assert_eq!(contents, expected);
//         Ok(())
//     }
// }