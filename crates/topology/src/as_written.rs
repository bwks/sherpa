//! A manifest as the user wrote it, which is what the server stores as a lab's definition.
//!
//! Before it sends a manifest, the client changes some of it. It replaces `ztp.config`, a path,
//! with the file's contents, and `KEY=$VAR` environment variables with their values; it reads
//! the files and scripts the manifest names, whose contents travel beside it as
//! [`crate::NodeFileContents`]; and it never serializes `provisioners` or
//! `provision_concurrency`. The server builds nodes from that
//! resolved manifest. A [`ManifestAsWritten`], sent beside it, carries those parts as written,
//! and [`Manifest::definition`] puts them back and drops every content, giving the manifest to
//! store: parameters as written, never file contents or resolved values.

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde_derive::{Deserialize, Serialize};

use crate::manifest::Manifest;
use crate::provisioner::Provisioner;

/// The parts of a manifest the client changes or does not serialize before sending it, as the
/// user wrote them: paths and settings, never the contents of a file or a variable's value.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ManifestAsWritten {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provision_concurrency: Option<u8>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub nodes: BTreeMap<String, NodeAsWritten>,
}

/// One node's parts of a [`ManifestAsWritten`].
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct NodeAsWritten {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ztp_config: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_variables: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provisioners: Option<Vec<Provisioner>>,
}

impl Manifest {
    /// The parts of this manifest, as the user wrote it, that the client changes or does not
    /// serialize before sending it. Taken before the client resolves its files and variables.
    #[must_use]
    pub fn as_written(&self) -> ManifestAsWritten {
        let mut nodes = BTreeMap::new();
        for node in &self.nodes {
            let parts = NodeAsWritten {
                ztp_config: node.ztp.config.clone(),
                environment_variables: node.environment_variables.clone(),
                provisioners: node.provisioners.clone(),
            };
            if parts != NodeAsWritten::default() {
                nodes.insert(node.name.clone(), parts);
            }
        }
        ManifestAsWritten {
            provision_concurrency: self.provision_concurrency,
            nodes,
        }
    }

    /// The manifest to store as the lab's definition: this one, which the client may have
    /// resolved, with the parts in `as_written` as the user wrote them, and with none of the
    /// file contents the client read (every `*_data` field). Without `as_written` (an older
    /// client), `ztp.config`, `environment_variables`, `provisioners` and
    /// `provision_concurrency` are left out rather than stored resolved.
    ///
    /// # Errors
    ///
    /// Fails when `as_written` names a node this manifest does not have.
    pub fn definition(&self, as_written: Option<&ManifestAsWritten>) -> Result<Manifest> {
        if let Some(name) = as_written.and_then(|written| {
            written
                .nodes
                .keys()
                .find(|name| !self.nodes.iter().any(|node| node.name == **name))
        }) {
            bail!("The manifest as written names node '{name}', which is not in the manifest");
        }
        let mut definition = self.clone();
        definition.provision_concurrency =
            as_written.and_then(|written| written.provision_concurrency);
        for node in &mut definition.nodes {
            let parts = as_written.and_then(|written| written.nodes.get(&node.name));
            node.ztp.config = parts.and_then(|parts| parts.ztp_config.clone());
            node.environment_variables =
                parts.and_then(|parts| parts.environment_variables.clone());
            node.provisioners = parts.and_then(|parts| parts.provisioners.clone());
            node.text_files_data = None;
            node.systemd_units_data = None;
            node.startup_scripts_data = None;
        }
        Ok(definition)
    }

    /// Whether the manifest holds any content the client read for its files and scripts: a
    /// `*_data` field. A lab's definition must not.
    #[must_use]
    pub fn has_file_contents(&self) -> bool {
        self.nodes.iter().any(|node| {
            node.text_files_data.is_some()
                || node.systemd_units_data.is_some()
                || node.startup_scripts_data.is_some()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{StartupScript, TextFileData};

    const MANIFEST: &str = r#"
name = "lab"
provision_concurrency = 2
[[nodes]]
name = "dev01"
model = "ubuntu_linux"
environment_variables = ["MODE=lab", "TOKEN=$LAB_TOKEN"]
startup_scripts = ["scripts/start.sh"]
ztp = { config = "configs/dev01.yaml" }
[[nodes.text_files]]
src = "files/motd"
dst = "/etc/motd"
user = "root"
group = "root"
permissions = 644
[[nodes.provisioners]]
type = "ssh"
scripts = ["./setup.sh"]
[[nodes]]
name = "dev02"
model = "ubuntu_linux"
"#;

    /// `manifest` as the client resolves it: its ZTP config and variables replaced, and the
    /// contents of its files added.
    fn resolved(mut manifest: Manifest) -> Manifest {
        let node = &mut manifest.nodes[0];
        node.ztp.config = Some("I2Nsb3VkLWNvbmZpZw==".to_owned());
        node.environment_variables = Some(vec!["MODE=lab".to_owned(), "TOKEN=s3cr3t".to_owned()]);
        node.startup_scripts_data = Some(vec![StartupScript {
            filename: "start.sh".to_owned(),
            content: "ZWNobw==".to_owned(),
        }]);
        node.text_files_data = Some(vec![TextFileData {
            content: "V2VsY29tZQ==".to_owned(),
            dst: "/etc/motd".to_owned(),
            ..TextFileData::default()
        }]);
        manifest
    }

    #[test]
    fn the_definition_is_the_manifest_as_written() {
        let written: Manifest = toml::from_str(MANIFEST).unwrap();
        let parts = written.as_written();
        assert_eq!(parts.nodes.keys().collect::<Vec<_>>(), ["dev01"]);

        // What reaches the server: the resolved manifest as JSON, without provisioners, and
        // the parts as written beside it.
        let sent: Manifest =
            serde_json::from_value(serde_json::to_value(resolved(written.clone())).unwrap())
                .unwrap();
        let parts: ManifestAsWritten =
            serde_json::from_value(serde_json::to_value(&parts).unwrap()).unwrap();

        let definition = sent.definition(Some(&parts)).unwrap();
        assert_eq!(definition, written);
        assert!(!definition.has_file_contents());
        let text = serde_json::to_string(&definition).unwrap();
        for content in ["s3cr3t", "I2Nsb3Vk", "ZWNobw", "V2VsY29tZQ"] {
            assert!(!text.contains(content), "{content}: {text}");
        }
    }

    #[test]
    fn without_the_parts_as_written_nothing_resolved_is_kept() {
        let written: Manifest = toml::from_str(MANIFEST).unwrap();
        let sent = resolved(written);
        assert!(sent.has_file_contents());
        let definition = sent.definition(None).unwrap();
        let node = &definition.nodes[0];
        assert_eq!(node.ztp.config, None);
        assert_eq!(node.environment_variables, None);
        assert_eq!(node.provisioners, None);
        assert_eq!(definition.provision_concurrency, None);
        assert!(!definition.has_file_contents());
        // References survive: the paths the manifest gives.
        assert_eq!(
            node.startup_scripts.as_deref(),
            Some(&["scripts/start.sh".to_owned()][..])
        );
        assert_eq!(node.text_files.as_ref().unwrap()[0].src, "files/motd");
    }

    #[test]
    fn parts_for_a_node_not_in_the_manifest_are_refused() {
        let manifest: Manifest = toml::from_str(MANIFEST).unwrap();
        let mut parts = manifest.as_written();
        parts
            .nodes
            .insert("ghost".to_owned(), NodeAsWritten::default());
        let error = manifest.definition(Some(&parts)).unwrap_err();
        assert!(error.to_string().contains("ghost"), "{error}");
    }
}
