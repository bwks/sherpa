use anyhow::{Result, ensure};
use tracing::instrument;

/// Refuse manifests whose nodes have provisioners.
///
/// The `sherpa` CLI runs provisioners itself and never sends them, so they only arrive here from
/// the web UI or the REST API, which cannot run them.
#[instrument(skip(manifest), level = "debug")]
pub fn reject_provisioners(manifest: &topology::Manifest) -> Result<()> {
    for node in &manifest.nodes {
        ensure!(
            node.provisioners.as_deref().unwrap_or_default().is_empty(),
            "Node '{}' has provisioners, which only the sherpa CLI can run: start the lab with \
             `sherpa up`, or remove them",
            node.name
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(provisioner: &str) -> topology::Manifest {
        toml::from_str(&format!(
            "name = \"lab\"\n[[nodes]]\nname = \"dev01\"\nmodel = \"ubuntu_linux\"\n{provisioner}"
        ))
        .unwrap()
    }

    #[test]
    fn manifests_with_provisioners_are_refused() {
        let manifest = manifest("[[nodes.provisioners]]\ntype = \"ssh\"\nscripts = [\"a.sh\"]\n");
        let error = reject_provisioners(&manifest).unwrap_err().to_string();
        assert!(error.contains("dev01"), "{error}");
        assert!(error.contains("sherpa up"), "{error}");
    }

    #[test]
    fn manifests_without_provisioners_pass() {
        reject_provisioners(&manifest("")).unwrap();
    }

    #[test]
    fn manifests_sent_by_the_client_never_carry_provisioners() {
        let manifest = manifest("[[nodes.provisioners]]\ntype = \"ssh\"\nscripts = [\"a.sh\"]\n");
        let sent: topology::Manifest =
            serde_json::from_value(serde_json::to_value(&manifest).unwrap()).unwrap();
        reject_provisioners(&sent).unwrap();
    }
}
