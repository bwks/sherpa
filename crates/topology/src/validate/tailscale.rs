use anyhow::{Result, ensure};
use tracing::instrument;

use crate::Manifest;

/// Validate configuration without resolving secrets or contacting Tailscale.
///
/// # Errors
///
/// Fails when an enabled tailnet does not name a valid environment variable for its key.
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
    use crate::TailscaleConfig;

    fn enabled(auth_key_env: Option<&str>) -> Manifest {
        Manifest {
            tailscale: Some(TailscaleConfig {
                enabled: true,
                auth_key_env: auth_key_env.map(str::to_owned),
            }),
            ..Manifest::default()
        }
    }

    #[test]
    fn lab_settings_004_an_enabled_tailnet_names_the_variable_that_holds_its_key() {
        let refused = [
            None,
            Some(""),
            Some("1KEY"),
            Some("$KEY"),
            Some("key-name"),
            Some("tskey-auth-secret"),
            Some("KEY=value"),
        ];
        for name in refused {
            let error = validate_tailscale(&enabled(name)).unwrap_err().to_string();
            assert!(
                error.contains("tailscale.auth_key_env must name an environment variable"),
                "{name:?}: {error}"
            );
        }
        validate_tailscale(&enabled(Some("SHERPA_TAILSCALE_KEY"))).unwrap();
    }
}
