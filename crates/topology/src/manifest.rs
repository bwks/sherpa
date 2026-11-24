use std::fs;

use anyhow::{Context, Result};
use serde_derive::{Deserialize, Serialize};

use super::bridge::Bridge;
use super::link::Link2;
use super::node::Node;
use super::talos_cluster::TalosCluster;
use shared::data::NodeModel;
use shared::util::{generate_lab_name, load_file as load_file_util};

#[derive(Clone, Debug, Deserialize, Serialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailscale: Option<TailscaleConfig>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready_timeout: Option<u64>,
    #[serde(default, skip_serializing)]
    pub provision_concurrency: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub talos_clusters: Option<Vec<TalosCluster>>,
    pub nodes: Vec<Node>,
    pub links: Option<Vec<Link2>>,
    pub bridges: Option<Vec<Bridge>>,
}

/// Optional per-lab tailnet enrollment. Secrets are resolved by the client.
#[derive(Clone, Debug, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TailscaleConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_key_env: Option<String>,
}

impl Manifest {
    /// An example manifest with a random name: two Linux nodes and a link between them.
    ///
    /// # Errors
    ///
    /// Fails when a lab name cannot be generated.
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
            src: format!("{}::eth1", dev01.name),
            dst: format!("{}::eth1", dev02.name),
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

    /// Write the manifest to `file_path` as TOML.
    ///
    /// # Errors
    ///
    /// Fails when the manifest cannot be serialized or the file cannot be written.
    pub fn write_file(&self, file_path: &str) -> Result<()> {
        let contents =
            toml::to_string_pretty(self).context("Failed to serialize manifest to TOML")?;
        fs::write(file_path, contents)
            .context(format!("Failed to write manifest file: {file_path}"))?;
        Ok(())
    }

    /// Read and parse the manifest at `file_path`.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or is not a valid manifest.
    pub fn load_file(file_path: &str) -> Result<Manifest> {
        let file_contents = load_file_util(file_path)?;
        toml::from_str(&file_contents)
            .with_context(|| format!("Failed to parse manifest {file_path}"))
    }
}

#[cfg(test)]
mod tests {
    use std::{env, process};

    use super::*;

    #[test]
    fn manifest_file_002_a_file_that_is_not_toml_is_refused_naming_the_file() {
        let path = env::temp_dir().join(format!("sherpa-bad-manifest-{}.toml", process::id()));
        fs::write(&path, "name = 'lab'\nnodes = [\n").unwrap();
        let path = path.to_str().unwrap();
        let error = format!("{:#}", Manifest::load_file(path).unwrap_err());
        fs::remove_file(path).unwrap();
        assert!(error.contains(path), "{error}");
    }

    #[test]
    fn manifest_file_007_a_written_manifest_has_a_table_only_for_the_groups_a_node_sets() {
        let manifest: Manifest = toml::from_str(
            "name='test'\n[[nodes]]\nname='vm'\nmodel='ubuntu_linux'\n[nodes.cpu]\ncount=2\n",
        )
        .unwrap();
        let text = toml::to_string_pretty(&manifest).unwrap();
        assert!(text.contains("[nodes.cpu]\ncount = 2"), "{text}");
        for group in [
            "numa",
            "boot_disk",
            "interfaces",
            "ztp",
            "ready_check",
            "vm",
            "container",
            "unikernel",
        ] {
            assert!(
                !text.contains(&format!("[nodes.{group}]")),
                "{group}: {text}"
            );
        }
        let restored: Manifest = toml::from_str(&text).unwrap();
        assert_eq!(restored, manifest);
    }
}
