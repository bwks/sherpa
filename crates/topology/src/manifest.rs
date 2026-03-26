use std::fs;

use anyhow::Result;
use serde_derive::{Deserialize, Serialize};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Value};

use super::bridge::Bridge;
use super::link::Link2;
use super::node::Node;
use shared::data::{ConfigurationManagement, NodeModel, ZtpServer};
use shared::util::load_file as load_file_util;

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready_timeout: Option<u64>,
    pub nodes: Vec<Node>,
    pub links: Option<Vec<Link2>>,
    pub bridges: Option<Vec<Bridge>>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ztp_server: Option<ZtpServer>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_management: Option<ConfigurationManagement>,
}

impl Manifest {
    pub fn example() -> Result<Self> {
        let name =
            petname::petname(2, "-").ok_or(anyhow::anyhow!("Failed to generate manifest name"))?;

        let dev01 = Node {
            name: "dev01".to_owned(),
            model: NodeModel::UbuntuLinux,
            ..Default::default()
        };
        let dev02 = Node {
            name: "dev02".to_owned(),
            model: NodeModel::FedoraLinux,
            ..Default::default()
        };

        let links = vec![Link2 {
            src: format!("{}::{}", dev01.name.clone(), "eth1"),
            dst: format!("{}::{}", dev02.name.clone(), "eth1"),
            p2p: None,
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

        doc["nodes"] = Item::Value(Value::Array(devices_array));

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
                link_table.insert("dst", Value::from(link.dst.as_str()));
                link_array.push_formatted(Value::from(link_table));
            }
            doc["links"] = Item::Value(Value::Array(link_array));
        }

        fs::write(file_path, doc.to_string())?;
        Ok(())
    }

    pub fn load_file(file_path: &str) -> Result<Manifest> {
        let file_contents = load_file_util(file_path)?;
        let manifest: Manifest = toml::from_str(&file_contents)?;
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_deserialize_ready_timeout() {
        let toml_str = r#"
name = "my-lab"
ready_timeout = 300

nodes = [
  { name = "dev01", model = "cisco_iosv" },
]
"#;
        let manifest: Manifest = toml::from_str(toml_str).expect("Failed to parse manifest");
        assert_eq!(manifest.name, "my-lab");
        assert_eq!(manifest.ready_timeout, Some(300));
        assert_eq!(manifest.nodes.len(), 1);
        assert_eq!(manifest.nodes[0].skip_ready_check, None);
    }

    #[test]
    fn test_manifest_deserialize_skip_ready_check() {
        let toml_str = r#"
name = "my-lab"

nodes = [
  { name = "dev01", model = "cisco_iosv" },
  { name = "dev02", model = "cisco_cat8000v", skip_ready_check = true },
]
"#;
        let manifest: Manifest = toml::from_str(toml_str).expect("Failed to parse manifest");
        assert_eq!(manifest.ready_timeout, None);
        assert_eq!(manifest.nodes.len(), 2);
        assert_eq!(manifest.nodes[0].skip_ready_check, None);
        assert_eq!(manifest.nodes[1].skip_ready_check, Some(true));
    }

    #[test]
    fn test_manifest_deserialize_ztp_config() {
        let toml_str = r#"
name = "my-lab"

nodes = [
  { name = "dev01", model = "cisco_cat8000v", ztp_config = "configs/dev01.txt" },
  { name = "dev02", model = "cisco_iosv" },
]
"#;
        let manifest: Manifest = toml::from_str(toml_str).expect("Failed to parse manifest");
        assert_eq!(manifest.nodes.len(), 2);
        assert_eq!(
            manifest.nodes[0].ztp_config,
            Some("configs/dev01.txt".to_string())
        );
        assert_eq!(manifest.nodes[1].ztp_config, None);
    }

    #[test]
    fn test_manifest_deserialize_all_ready_check_fields() {
        let toml_str = r#"
name = "my-lab"
ready_timeout = 120

nodes = [
  { name = "dev01", model = "cisco_iosv" },
  { name = "dev02", model = "cisco_cat8000v", skip_ready_check = true },
  { name = "dev03", model = "ubuntu_linux", skip_ready_check = false },
]
"#;
        let manifest: Manifest = toml::from_str(toml_str).expect("Failed to parse manifest");
        assert_eq!(manifest.ready_timeout, Some(120));
        assert_eq!(manifest.nodes[0].skip_ready_check, None);
        assert_eq!(manifest.nodes[1].skip_ready_check, Some(true));
        assert_eq!(manifest.nodes[2].skip_ready_check, Some(false));
    }
}
