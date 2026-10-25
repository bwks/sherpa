//! The Tailscale auth key, read from the client environment and never written to the manifest.
use anyhow::{Context, Result, bail};
use shared::data::{ClientConfig, TailnetAuthKey};
use shared::validators::validate_tailscale_key;
use topology::Manifest;
use topology::validate::validate_tailscale;

/// Resolve the enrollment key separately so the manifest never contains it.
#[tracing::instrument(skip(manifest, config), level = "debug")]
pub(crate) fn resolve_tailscale_key(
    manifest: &Manifest,
    server_url: &str,
    config: &ClientConfig,
) -> Result<Option<TailnetAuthKey>> {
    resolve_tailscale_key_using(manifest, server_url, config, |name| {
        std::env::var(name).ok()
    })
}

/// The key a resume sends. `down` logs the gateway out, so a whole-lab resume of a Tailscale
/// lab enrolls it again; a single-node resume leaves the gateway alone and sends none.
/// Without the lab's manifest there is no key to send, and the server explains if one is needed.
#[tracing::instrument(skip(manifest, config), level = "debug")]
pub(crate) fn resolve_resume_tailscale_key(
    node_name: Option<&str>,
    manifest: Option<&Manifest>,
    server_url: &str,
    config: &ClientConfig,
) -> Result<Option<TailnetAuthKey>> {
    match (node_name, manifest) {
        (None, Some(manifest)) => resolve_tailscale_key(manifest, server_url, config),
        (Some(_), _) | (None, None) => Ok(None),
    }
}

fn resolve_tailscale_key_using(
    manifest: &Manifest,
    server_url: &str,
    config: &ClientConfig,
    lookup: impl FnOnce(&str) -> Option<String>,
) -> Result<Option<TailnetAuthKey>> {
    validate_tailscale(manifest)?;
    let Some(tailscale) = manifest.tailscale.as_ref().filter(|c| c.enabled) else {
        return Ok(None);
    };
    // WebSocketClient always validates its custom CA or TOFU trust store unless insecure.
    if !server_url.starts_with("wss://") || config.server_connection.insecure {
        bail!("Tailscale enrollment requires WSS with certificate validation enabled");
    }
    let name = tailscale
        .auth_key_env
        .as_deref()
        .context("Missing tailscale.auth_key_env")?;
    let key = lookup(name).with_context(|| {
        format!(
            "Tailscale auth-key environment variable '{name}' is not set or is not valid Unicode"
        )
    })?;
    validate_tailscale_key(&key)?;
    Ok(Some(TailnetAuthKey::new(key)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAILNET_MANIFEST: &str =
        "name='test'\nnodes=[]\n[tailscale]\nenabled=true\nauth_key_env='SHERPA_TEST_NEVER_SET'";

    #[test]
    fn tailnet_key_resolution_requires_secure_transport_and_never_changes_manifest() {
        let manifest: Manifest =
            toml::from_str("name='test'\nnodes=[]\n[tailscale]\nenabled=true\nauth_key_env='KEY'")
                .unwrap();
        let mut config = ClientConfig::default();
        let key = "tskey-auth-testfixture";
        let resolved =
            resolve_tailscale_key_using(&manifest, "wss://example.invalid", &config, |_| {
                Some(key.into())
            })
            .unwrap()
            .unwrap();
        assert_eq!(resolved.expose(), key);
        assert!(!serde_json::to_string(&manifest).unwrap().contains(key));
        assert!(
            resolve_tailscale_key_using(&manifest, "wss://example.invalid", &config, |_| None)
                .is_err()
        );
        assert!(
            resolve_tailscale_key_using(&manifest, "ws://example.invalid", &config, |_| Some(
                key.into()
            ))
            .is_err()
        );
        config.server_connection.insecure = true;
        assert!(
            resolve_tailscale_key_using(&manifest, "wss://example.invalid", &config, |_| Some(
                key.into()
            ))
            .is_err()
        );
        let disabled = Manifest::default();
        assert!(
            resolve_tailscale_key_using(&disabled, "ws://example.invalid", &config, |_| panic!(
                "disabled tailnet must not resolve secrets"
            ))
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn whole_lab_resume_of_a_tailnet_lab_needs_the_key() {
        let manifest: Manifest = toml::from_str(TAILNET_MANIFEST).unwrap();
        let config = ClientConfig::default();
        let error =
            resolve_resume_tailscale_key(None, Some(&manifest), "wss://example.invalid", &config)
                .unwrap_err();
        assert!(error.to_string().contains("SHERPA_TEST_NEVER_SET"));
        assert!(
            resolve_resume_tailscale_key(None, Some(&manifest), "ws://example.invalid", &config)
                .is_err()
        );
    }

    #[test]
    fn single_node_resume_and_missing_manifest_send_no_key() {
        let manifest: Manifest = toml::from_str(TAILNET_MANIFEST).unwrap();
        let config = ClientConfig::default();
        assert!(
            resolve_resume_tailscale_key(
                Some("dev01"),
                Some(&manifest),
                "ws://example.invalid",
                &config
            )
            .unwrap()
            .is_none()
        );
        assert!(
            resolve_resume_tailscale_key(None, None, "ws://example.invalid", &config)
                .unwrap()
                .is_none()
        );
    }
}
