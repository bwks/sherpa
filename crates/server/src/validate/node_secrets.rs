use std::collections::{BTreeMap, HashSet};

use anyhow::{Context, Result, ensure};
use tracing::instrument;

use shared::data::{ModelKind, NodeKind, SecretFile};
use shared::validators::validate_secret_file;

use crate::services::error::ServiceError;

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
        let node = node.ok_or_else(|| ServiceError::SecretsForUnknownNode {
            node: node_name.clone(),
        })?;
        ensure!(
            ModelKind::new(node.model, node.kind)?.kind() == NodeKind::Container,
            ServiceError::SecretsForNonContainer {
                node: node_name.clone()
            }
        );
        validate_secret_files(files).context(ServiceError::NodeSecretFiles {
            node: node_name.clone(),
        })?;
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
            ServiceError::SecretFileNameTwice {
                name: file.name.clone()
            }
        );
        file.content
            .decode()
            .with_context(|| ServiceError::SecretFileNotBase64 {
                name: file.name.clone(),
            })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::SecretContent;
    use shared::error::SharedError;

    fn manifest() -> topology::Manifest {
        toml::from_str(
            r#"
            name = "lab"

            [[nodes]]
            name = "db01"
            model = "mongo_db"

            [[nodes]]
            name = "dev01"
            model = "ubuntu_linux"
            "#,
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

    fn error(secrets: &BTreeMap<String, Vec<SecretFile>>) -> ServiceError {
        validate_node_secrets(&manifest(), secrets)
            .unwrap_err()
            .downcast::<ServiceError>()
            .expect("a service error")
    }

    #[test]
    fn container_secrets_010_secrets_for_a_node_not_in_the_manifest_or_not_a_container_are_refused()
    {
        assert_eq!(
            error(&secrets("db09", vec![file("password", 444)])),
            ServiceError::SecretsForUnknownNode {
                node: "db09".to_owned()
            }
        );
        assert_eq!(
            error(&secrets("dev01", vec![file("password", 444)])),
            ServiceError::SecretsForNonContainer {
                node: "dev01".to_owned()
            }
        );
    }

    #[test]
    fn container_secrets_011_a_bad_file_is_refused_carrying_the_node_and_good_files_are_taken() {
        let bad_content = SecretFile {
            content: serde_json::from_value(serde_json::json!("!!")).unwrap(),
            ..file("password", 444)
        };
        let bad_files = [
            vec![file("../x", 444)],
            vec![file("password", 448)],
            vec![file("a", 444), file("a", 400)],
            vec![bad_content.clone()],
        ];
        for files in bad_files {
            assert_eq!(
                error(&secrets("db01", files)),
                ServiceError::NodeSecretFiles {
                    node: "db01".to_owned()
                }
            );
        }
        let mode = validate_secret_files(&[file("password", 448)]).unwrap_err();
        assert_eq!(
            mode.downcast_ref::<SharedError>(),
            Some(&SharedError::ModeNotOctal { digits: 448 })
        );
        let twice = validate_secret_files(&[file("a", 444), file("a", 400)]).unwrap_err();
        assert_eq!(
            twice.downcast_ref::<ServiceError>(),
            Some(&ServiceError::SecretFileNameTwice {
                name: "a".to_owned()
            })
        );
        let content = validate_secret_files(&[bad_content]).unwrap_err();
        assert_eq!(
            content.downcast_ref::<ServiceError>(),
            Some(&ServiceError::SecretFileNotBase64 {
                name: "password".to_owned()
            })
        );
        validate_node_secrets(&manifest(), &BTreeMap::new()).unwrap();
        validate_node_secrets(
            &manifest(),
            &secrets("db01", vec![file("password", 444), file("key", 400)]),
        )
        .unwrap();
    }
}
