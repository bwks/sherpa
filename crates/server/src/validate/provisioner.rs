use anyhow::{Result, ensure};
use tracing::instrument;

use crate::services::error::ServiceError;

/// Refuse manifests whose nodes have provisioners.
///
/// The `sherpa` CLI runs provisioners itself and never sends them, so they only arrive here from
/// the web UI or the REST API, which cannot run them.
#[instrument(skip(manifest), level = "debug")]
pub fn reject_provisioners(manifest: &topology::Manifest) -> Result<()> {
    for node in &manifest.nodes {
        ensure!(
            node.provisioners.as_deref().unwrap_or_default().is_empty(),
            ServiceError::NodeHasProvisioners {
                node: node.name.clone()
            }
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SSH_PROVISIONER: &str = r#"
        [[nodes.provisioners]]
        type = "ssh"
        scripts = ["a.sh"]
    "#;

    fn manifest(provisioner: &str) -> topology::Manifest {
        toml::from_str(&format!(
            r#"
            name = "lab"

            [[nodes]]
            name = "dev01"
            model = "ubuntu_linux"
            {provisioner}
            "#
        ))
        .unwrap()
    }

    #[test]
    fn provisioner_secrecy_002_a_manifest_with_provisioners_is_refused_and_one_without_is_taken() {
        let error = reject_provisioners(&manifest(SSH_PROVISIONER)).unwrap_err();
        assert_eq!(
            error.downcast_ref::<ServiceError>(),
            Some(&ServiceError::NodeHasProvisioners {
                node: "dev01".to_owned()
            })
        );
        reject_provisioners(&manifest("")).unwrap();
    }

    #[test]
    fn provisioner_secrecy_001_the_manifest_the_cli_sends_has_no_provisioners_and_is_taken() {
        let manifest = manifest(SSH_PROVISIONER);
        let sent_json = serde_json::to_string(&manifest).unwrap();
        assert!(!sent_json.contains("provisioners"), "{sent_json}");
        assert!(!sent_json.contains("a.sh"), "{sent_json}");
        let sent: topology::Manifest = serde_json::from_str(&sent_json).unwrap();
        reject_provisioners(&sent).unwrap();
    }
}
