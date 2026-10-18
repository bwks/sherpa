use anyhow::{Result, ensure};
use tracing::instrument;

use crate::Manifest;

/// Validate configuration without resolving secrets or contacting Tailscale.
#[instrument(skip(manifest), level = "debug")]
pub fn validate_tailscale(manifest: &Manifest) -> Result<()> {
    let Some(config) = &manifest.tailscale else {
        return Ok(());
    };
    if !config.enabled {
        return Ok(());
    }
    let name = config.auth_key_env.as_deref().unwrap_or_default();
    ensure!(
        valid_env_name(name),
        "tailscale.auth_key_env must name an environment variable (letters, digits, underscores; not starting with a digit)"
    );
    Ok(())
}

fn valid_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_reference_is_a_name_not_a_secret_or_expression() {
        for name in [
            "",
            "1KEY",
            "$KEY",
            "key-name",
            "tskey-auth-secret",
            "KEY=value",
        ] {
            assert!(!valid_env_name(name));
        }
        assert!(valid_env_name("SHERPA_TAILSCALE_KEY"));
    }
}
