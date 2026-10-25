//! The container provisioner: the CLI reads its secret files and sends them to the server
//! outside the manifest, and the server mounts them in the container at `/run/secrets`.
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use shared::data::{ProvisionResult, ProvisionStatus, SecretContent, SecretFile};
use shared::konst::CONTAINER_SECRETS_DIR;
use topology::{Manifest, Node, Provisioner};

use super::client_files::existing_client_file;

/// The secret files of every container node with a container provisioner, by node name.
pub fn container_secrets(
    manifest: &Manifest,
    manifest_dir: &Path,
) -> Result<BTreeMap<String, Vec<SecretFile>>> {
    let mut secrets = BTreeMap::new();
    for node in &manifest.nodes {
        let files = node_secrets(node, manifest_dir)?;
        if !files.is_empty() {
            secrets.insert(node.name.clone(), files);
        }
    }
    Ok(secrets)
}

/// A results row for each container node whose secret files the server mounted.
pub fn container_results(secrets: &BTreeMap<String, Vec<SecretFile>>) -> Vec<ProvisionResult> {
    let mut results = Vec::with_capacity(secrets.len());
    for (node, files) in secrets {
        results.push(ProvisionResult {
            node: node.clone(),
            status: ProvisionStatus::Provisioned,
            detail: format!("{} secret files in {CONTAINER_SECRETS_DIR}", files.len()),
        });
    }
    results
}

/// A node's secret files from its container provisioners, read from the client.
pub fn node_secrets(node: &Node, manifest_dir: &Path) -> Result<Vec<SecretFile>> {
    let mut secrets = Vec::new();
    for provisioner in node.provisioners.as_deref().unwrap_or_default() {
        let Provisioner::Container(container) = provisioner else {
            continue;
        };
        for file in &container.files {
            let path = existing_client_file(manifest_dir, &file.src)?;
            let content = fs::read(&path)
                .with_context(|| format!("Cannot read secret file {}", path.display()))?;
            secrets.push(SecretFile {
                name: file.name.clone(),
                mode: file.mode,
                content: SecretContent::from_bytes(&content),
            });
        }
    }
    Ok(secrets)
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    fn manifest() -> Manifest {
        toml::from_str(
            r#"
name = "lab"

[[nodes]]
name = "db01"
model = "mongo_db"

[[nodes.provisioners]]
type = "container"

[[nodes.provisioners.files]]
src = "secrets/password"
name = "mongo_password"

[[nodes.provisioners.files]]
src = "secrets/tls.key"
name = "tls.key"
mode = 400

[[nodes]]
name = "dev01"
model = "ubuntu_linux"

[[nodes.provisioners]]
type = "ssh"
scripts = ["setup.sh"]

[[nodes]]
name = "bus01"
model = "nats_bus"
"#,
        )
        .unwrap()
    }

    #[test]
    fn secret_files_are_read_for_container_provisioners_only() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("secrets")).unwrap();
        fs::write(dir.path().join("secrets/password"), "pw").unwrap();
        fs::write(dir.path().join("secrets/tls.key"), [0u8, 255]).unwrap();

        let secrets = container_secrets(&manifest(), dir.path()).unwrap();
        assert_eq!(secrets.keys().collect::<Vec<_>>(), vec!["db01"]);
        let files = &secrets["db01"];
        assert_eq!(files[0].name, "mongo_password");
        assert_eq!(files[0].mode, 444);
        assert_eq!(files[0].content.decode().unwrap(), b"pw");
        assert_eq!(files[1].name, "tls.key");
        assert_eq!(files[1].mode, 400);
        assert_eq!(files[1].content.decode().unwrap(), [0u8, 255]);
    }

    #[test]
    fn mounted_secrets_are_reported_per_node() {
        let mut secrets = BTreeMap::new();
        secrets.insert(
            "db01".to_string(),
            vec![SecretFile {
                name: "token".to_string(),
                mode: 444,
                content: SecretContent::from_bytes(b"t"),
            }],
        );
        let results = container_results(&secrets);
        assert_eq!(results[0].node, "db01");
        assert_eq!(results[0].status, ProvisionStatus::Provisioned);
        assert_eq!(results[0].detail, "1 secret files in /run/secrets");
    }

    #[test]
    fn a_node_without_container_provisioners_has_no_secrets() {
        let dir = tempdir().unwrap();
        let manifest = manifest();
        assert!(
            node_secrets(&manifest.nodes[1], dir.path())
                .unwrap()
                .is_empty()
        );
        assert!(
            node_secrets(&manifest.nodes[2], dir.path())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_missing_secret_file_is_an_error_naming_it() {
        let dir = tempdir().unwrap();
        let error = format!(
            "{:#}",
            container_secrets(&manifest(), dir.path()).unwrap_err()
        );
        assert!(error.contains("secrets/password"), "{error}");
    }
}
