use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;
use tracing::instrument;

use crate::konst::SHERPA_LAB_MANIFEST_FILE;

/// Load the resolved manifest, retaining read compatibility with older labs.
#[instrument(level = "debug")]
pub fn load_saved_manifest(lab_dir: &Path) -> Result<Value> {
    match fs::read_to_string(lab_dir.join(SHERPA_LAB_MANIFEST_FILE)) {
        Ok(contents) => {
            let value: toml::Value =
                toml::from_str(&contents).context("Invalid saved TOML manifest")?;
            serde_json::to_value(value).context("Unable to convert saved manifest for the API")
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let contents = fs::read_to_string(lab_dir.join("manifest.json"))
                .context("Unable to read saved lab manifest")?;
            serde_json::from_str(&contents).context("Invalid legacy lab manifest")
        }
        Err(error) => Err(error).context("Unable to read saved TOML manifest"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn prefers_toml_and_reads_legacy_manifests_only_when_absent() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("manifest.json"),
            r#"{"name":"legacy","nodes":[]}"#,
        )
        .unwrap();
        assert_eq!(load_saved_manifest(dir.path()).unwrap()["name"], "legacy");
        fs::write(
            dir.path().join(SHERPA_LAB_MANIFEST_FILE),
            "name='current'\nnodes=[]\n[tailscale]\nenabled=true\nauth_key_env='KEY'",
        )
        .unwrap();
        let current = load_saved_manifest(dir.path()).unwrap();
        assert_eq!(current["name"], "current");
        assert_eq!(current["tailscale"]["auth_key_env"], "KEY");
        fs::write(dir.path().join(SHERPA_LAB_MANIFEST_FILE), "invalid").unwrap();
        assert!(load_saved_manifest(dir.path()).is_err());
    }
}
