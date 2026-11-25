//! The contents of the files and scripts a manifest names, which the client reads and sends
//! beside the manifest, by node name. A manifest never holds them as keys: the user writes the
//! paths, and the server puts the contents into its nodes once it has read the manifest.

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde_derive::{Deserialize, Serialize};

use crate::TopologyError;
use crate::manifest::Manifest;
use crate::node::{StartupScript, SystemdUnitData, TextFileData};

/// One node's file contents, as the client read them.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NodeFileContents {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_files: Option<Vec<TextFileData>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub systemd_units: Option<Vec<SystemdUnitData>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_scripts: Option<Vec<StartupScript>>,
}

impl Manifest {
    /// The file contents the client read into this manifest's nodes, by node name, leaving out
    /// a node with none.
    #[must_use]
    pub fn file_contents(&self) -> BTreeMap<String, NodeFileContents> {
        let mut contents = BTreeMap::new();
        for node in &self.nodes {
            let files = NodeFileContents {
                text_files: node.text_files_data.clone(),
                systemd_units: node.systemd_units_data.clone(),
                startup_scripts: node.startup_scripts_data.clone(),
            };
            if files != NodeFileContents::default() {
                contents.insert(node.name.clone(), files);
            }
        }
        contents
    }

    /// Put the file contents sent beside this manifest into its nodes.
    ///
    /// # Errors
    ///
    /// Fails when `contents` names a node this manifest does not have.
    pub fn put_file_contents(
        &mut self,
        mut contents: BTreeMap<String, NodeFileContents>,
    ) -> Result<()> {
        for node in &mut self.nodes {
            let files = contents.remove(&node.name).unwrap_or_default();
            node.text_files_data = files.text_files;
            node.systemd_units_data = files.systemd_units;
            node.startup_scripts_data = files.startup_scripts;
        }
        if let Some(name) = contents.keys().next() {
            bail!(TopologyError::FileContentsUnknownNode { node: name.clone() });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value, to_value};

    use super::*;

    const MANIFEST: &str = r#"
name = "lab"
[[nodes]]
name = "dev01"
model = "ubuntu_linux"
startup_scripts = ["start.sh"]
[[nodes]]
name = "dev02"
model = "ubuntu_linux"
"#;

    #[test]
    fn client_content_015_the_contents_read_travel_beside_the_manifest_into_their_nodes() {
        let mut read: Manifest = toml::from_str(MANIFEST).unwrap();
        read.nodes[0].startup_scripts_data = Some(vec![StartupScript {
            filename: "start.sh".to_owned(),
            content: "ZWNobyBoaQ==".to_owned(),
        }]);
        let sent_manifest = to_value(&read).unwrap();
        let sent_contents = to_value(read.file_contents()).unwrap();

        let mut received: Manifest = from_value(sent_manifest).unwrap();
        assert_eq!(received.nodes[0].startup_scripts_data, None);
        received
            .put_file_contents(from_value(sent_contents).unwrap())
            .unwrap();
        assert_eq!(received, read);
    }

    #[test]
    fn client_content_016_contents_for_a_node_the_manifest_does_not_have_are_refused() {
        let mut manifest: Manifest = toml::from_str(MANIFEST).unwrap();
        let contents = BTreeMap::from([("dev03".to_owned(), NodeFileContents::default())]);
        let error = manifest.put_file_contents(contents).unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::FileContentsUnknownNode {
                node: "dev03".to_owned(),
            })
        );
    }
}
