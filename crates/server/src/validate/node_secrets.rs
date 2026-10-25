use std::collections::{BTreeMap, HashSet};

use anyhow::{Context, Result, ensure};
use tracing::instrument;

use shared::data::{NodeConfig, NodeKind, SecretFile};
use shared::validators::validate_secret_file;

/// Check container secret files sent with a lab or a redeploy: each is for a container node in
/// the manifest, and has a valid, unique name, a valid mode and base64 content.
#[instrument(skip(manifest, secrets), level = "debug")]
pub fn validate_node_secrets(
    manifest: &topology::Manifest,
    secrets: &BTreeMap<String, Vec<SecretFile>>,
) -> Result<()> {
    for (node_name, files) in secrets {
        let mut node = None;
        for candidate in &manifest.nodes {
            if candidate.name == *node_name {
                node = Some(candidate);
            }
        }
        let node = node.with_context(|| {
            format!("Secret files were sent for node '{node_name}', which is not in the manifest")
        })?;
        ensure!(
            NodeConfig::get_model(node.model).kind == NodeKind::Container,
            "Secret files were sent for node '{node_name}', but only containers take them"
        );
        validate_secret_files(files).with_context(|| format!("Node '{node_name}'"))?;
    }
    Ok(())
}

/// Check a node's secret files: valid, unique names, valid modes and base64 content.
#[instrument(skip(files), level = "debug")]
pub fn validate_secret_files(files: &[SecretFile]) -> Result<()> {
    let mut names = HashSet::new();
    for file in files {
        validate_secret_file(&file.name, file.mode)?;
        ensure!(
            names.insert(file.name.as_str()),
            "Two secret files are named '{}'",
            file.name
        );
        file.content
            .decode()
            .with_context(|| format!("Secret file '{}' is not valid base64", file.name))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::SecretContent;

    fn manifest() -> topology::Manifest {
        toml::from_str(
            "name = \"lab\"\n[[nodes]]\nname = \"db01\"\nmodel = \"mongo_db\"\n\
             [[nodes]]\nname = \"dev01\"\nmodel = \"ubuntu_linux\"\n",
        )
        .unwrap()
    }

    fn secrets(node: &str, files: Vec<SecretFile>) -> BTreeMap<String, Vec<SecretFile>> {
        BTreeMap::from([(node.to_string(), files)])
    }

    fn file(name: &str, mode: u32) -> SecretFile {
        SecretFile {
            name: name.to_string(),
            mode,
            content: SecretContent::from_bytes(b"x"),
        }
    }

    fn error(secrets: BTreeMap<String, Vec<SecretFile>>) -> String {
        format!(
            "{:#}",
            validate_node_secrets(&manifest(), &secrets).unwrap_err()
        )
    }

    #[test]
    fn valid_secrets_for_a_container_pass() {
        validate_node_secrets(&manifest(), &BTreeMap::new()).unwrap();
        validate_node_secrets(
            &manifest(),
            &secrets("db01", vec![file("password", 444), file("key", 400)]),
        )
        .unwrap();
    }

    #[test]
    fn secrets_must_be_for_container_nodes_in_the_manifest() {
        let missing = error(secrets("db09", vec![file("password", 444)]));
        assert!(
            missing.contains("db09") && missing.contains("not in the manifest"),
            "{missing}"
        );
        let vm = error(secrets("dev01", vec![file("password", 444)]));
        assert!(vm.contains("dev01") && vm.contains("container"), "{vm}");
    }

    #[test]
    fn bad_files_are_rejected() {
        let bad_content = SecretFile {
            content: serde_json::from_value(serde_json::json!("!!")).unwrap(),
            ..file("password", 444)
        };
        for (files, expected) in [
            (vec![file("../x", 444)], "../x"),
            (vec![file("password", 448)], "448"),
            (vec![file("a", 444), file("a", 400)], "Two secret files"),
            (vec![bad_content], "base64"),
        ] {
            let message = error(secrets("db01", files));
            assert!(message.contains(expected), "{expected}: {message}");
            assert!(message.contains("db01"), "{message}");
        }
    }
}
