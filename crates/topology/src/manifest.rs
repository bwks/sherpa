use std::fs;

use anyhow::Result;
use serde_derive::{Deserialize, Serialize};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Value};

use super::bridge::Bridge;
use super::link::Link2;
use super::node::Node;
use shared::data::{ConfigurationManagement, NodeModel, ZtpServer};
use shared::util::{generate_lab_name, load_file as load_file_util};

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Provision the authenticated lab owner's Linux account on supported VMs.
    #[serde(default)]
    pub provision_current_user: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailscale: Option<TailscaleConfig>,
    pub name: String,
    /// Optional blurb describing the lab's purpose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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

/// Optional per-lab tailnet enrollment. Secrets are resolved by the client.
#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct TailscaleConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_key_env: Option<String>,
}

impl Manifest {
    pub fn example() -> Result<Self> {
        let name = generate_lab_name()?;

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
            impairment: None,
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
        if let Some(description) = &self.description {
            doc["description"] = Item::Value(Value::from(description.as_str()));
        }
        doc["provision_current_user"] = Item::Value(Value::from(self.provision_current_user));
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
            if let Some(method) = device.boot_disk_clone_method {
                device_table.insert("boot_disk_clone_method", Value::from(method.to_string()));
            }
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

        if let Some(tailscale) = &self.tailscale {
            doc["tailscale"]["enabled"] = Item::Value(Value::from(tailscale.enabled));
            if let Some(name) = &tailscale.auth_key_env {
                doc["tailscale"]["auth_key_env"] = Item::Value(Value::from(name.as_str()));
            }
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
    use std::{env, process};

    use super::*;

    #[test]
    fn description_round_trip_preserves_multiline_text() {
        let input = r#"name = 'routing-lab'
description = '''Practice BGP routing.
Include "quoted" text, C:\configs, and Unicode: café.'''
nodes = []
"#;
        let manifest: Manifest = toml::from_str(input).unwrap();
        let saved = toml::to_string_pretty(&manifest).unwrap();
        let restored: Manifest = toml::from_str(&saved).unwrap();
        let original: toml::Value = toml::from_str(input).unwrap();
        let restored = toml::Value::try_from(&restored).unwrap();
        assert_eq!(restored["description"], original["description"]);
    }

    #[test]
    fn description_is_optional_and_omitted_when_absent() {
        let manifest: Manifest = toml::from_str("name='test'\nnodes=[]").unwrap();
        assert!(
            toml::Value::try_from(&manifest)
                .unwrap()
                .get("description")
                .is_none()
        );
    }

    #[test]
    fn description_rejects_non_string_values() {
        for value in ["true", "42", "[]", "{}"] {
            let input = format!("name='test'\nnodes=[]\ndescription={value}");
            assert!(toml::from_str::<Manifest>(&input).is_err());
        }
    }

    #[test]
    fn description_survives_manifest_file_write() {
        let path = env::temp_dir().join(format!("sherpa-description-{}.toml", process::id()));
        let path = path.to_str().unwrap();
        for description in [
            Some("Lab purpose: \"routing\"\nUnicode: café"),
            Some(""),
            None,
        ] {
            let mut input = toml::from_str::<toml::Value>("name='test'\nnodes=[]").unwrap();
            if let Some(description) = description {
                input.as_table_mut().unwrap().insert(
                    "description".into(),
                    toml::Value::String(description.into()),
                );
            }
            let manifest: Manifest = input.try_into().unwrap();
            manifest.write_file(path).unwrap();
            let restored = Manifest::load_file(path).unwrap();
            let value = toml::Value::try_from(&restored).unwrap();
            assert_eq!(
                value.get("description").and_then(toml::Value::as_str),
                description
            );
        }
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn current_user_flag_round_trip() {
        let manifest: Manifest =
            toml::from_str("name='test'\nnodes=[]\nprovision_current_user=true").unwrap();
        let saved = toml::to_string(&manifest).unwrap();
        assert_eq!(
            toml::from_str::<toml::Value>(&saved).unwrap()["provision_current_user"].as_bool(),
            Some(true)
        );
    }

    #[test]
    fn current_user_flag_defaults_false_and_rejects_overrides() {
        let manifest: Manifest = toml::from_str("name='test'\nnodes=[]").unwrap();
        assert!(!manifest.provision_current_user);
        for input in [
            "name='test'\nnodes=[]\nprovision_current_user='true'",
            "name='test'\nnodes=[]\nprovision_username='alice'",
            "name='test'\nnodes=[{name='vm',model='ubuntu_linux',provision_current_user=true}]",
        ] {
            assert!(toml::from_str::<Manifest>(input).is_err());
        }
    }

    #[test]
    fn lab_level_boot_disk_clone_method_is_rejected() {
        assert!(
            toml::from_str::<Manifest>("name='test'\nnodes=[]\nboot_disk_clone_method='overlay'")
                .is_err()
        );
    }

    #[test]
    fn disk_method_defaults_and_unknown_values() {
        let manifest: Manifest =
            toml::from_str("name='test'\nnodes=[{name='vm',model='ubuntu_linux'}]").unwrap();
        assert_eq!(manifest.nodes[0].boot_disk_clone_method, None);
        assert!(
            toml::from_str::<Manifest>(
                "name='test'\nnodes=[{name='vm',model='ubuntu_linux',boot_disk_clone_method='snapshot'}]"
            )
            .is_err()
        );
    }

    #[test]
    fn boot_disk_clone_method_round_trip() {
        let manifest: Manifest = toml::from_str(
            "name='test'\nnodes=[{name='vm',model='ubuntu_linux',boot_disk_clone_method='copy'}]",
        )
        .unwrap();
        let value = toml::Value::try_from(&manifest).unwrap();
        assert!(value.get("boot_disk_clone_method").is_none());
        assert_eq!(
            value["nodes"][0]["boot_disk_clone_method"].as_str(),
            Some("copy")
        );
    }

    #[test]
    fn tailscale_manifest_round_trip() {
        let input = "name = 'tailnet-lab'\nnodes = []\n[tailscale]\nenabled = true\nauth_key_env = 'SHERPA_TAILSCALE_KEY'\n";
        let manifest: Manifest = toml::from_str(input).expect("tailnet configuration must parse");
        let value = toml::Value::try_from(&manifest).unwrap();
        assert_eq!(value["tailscale"]["enabled"].as_bool(), Some(true));
        assert_eq!(
            value["tailscale"]["auth_key_env"].as_str(),
            Some("SHERPA_TAILSCALE_KEY")
        );
    }

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
    #[test]
    fn disabled_and_legacy_manifests_do_not_require_auth_keys() {
        let legacy: Manifest = toml::from_str("name='test'\nnodes=[]").unwrap();
        assert!(legacy.tailscale.is_none());
        let disabled: Manifest =
            toml::from_str("name='test'\nnodes=[]\n[tailscale]\nenabled=false").unwrap();
        assert!(!disabled.tailscale.unwrap().enabled);
        assert!(
            toml::from_str::<Manifest>(
                "name='test'\nnodes=[]\n[tailscale]\nenabled=true\nauth_key='do-not-store-secrets'"
            )
            .is_err()
        );
    }
    #[test]
    fn resolved_manifest_serializes_to_toml_with_tailnet_reference() {
        let manifest: Manifest = toml::from_str("name='test'\nnodes=[{name='r1',model='cisco_iosv'}]\n[tailscale]\nenabled=true\nauth_key_env='KEY'").unwrap();
        let saved = toml::to_string_pretty(&manifest).unwrap();
        let restored: Manifest = toml::from_str(&saved).unwrap();
        assert_eq!(restored.nodes.len(), 1);
        assert_eq!(
            restored.tailscale.unwrap().auth_key_env.as_deref(),
            Some("KEY")
        );
    }
}
