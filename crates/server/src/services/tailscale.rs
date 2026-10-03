//! Tailnet orchestration; callers authenticate and authorize the lab first.
use anyhow::{Context, Result, ensure};
use container::tailscale::{self, GatewayConfig};
use shared::data::{DbLab, LabInfo, TailnetAuthKey, TailnetStatus};
use shared::konst::SHERPA_MANAGEMENT_NETWORK_NAME;
use shared::util::{get_ipv4_addr, get_ipv6_addr};
use topology::Manifest;
use tracing::instrument;

use crate::daemon::state::AppState;

#[instrument(skip(manifest, key), level = "debug")]
pub(crate) fn validate_enrollment(
    manifest: &Manifest,
    key: Option<&TailnetAuthKey>,
    tls_enabled: bool,
) -> Result<bool> {
    validate::validate_tailscale(manifest)?;
    let enabled = manifest.tailscale.as_ref().is_some_and(|c| c.enabled);
    if enabled {
        ensure!(tls_enabled, "Tailscale enrollment requires server TLS");
        let key = key.context("Tailscale is enabled but no auth key was supplied; use the CLI with tailscale.auth_key_env")?;
        validate::validate_tailscale_key(key.expose())?;
    } else {
        ensure!(
            key.is_none(),
            "Tailscale credential supplied without enabling tailscale in the manifest"
        );
    }
    Ok(enabled)
}

#[instrument(skip(state, key, info, lab), fields(lab_id = %lab.lab_id))]
pub(crate) async fn provision(
    state: &AppState,
    lab: &DbLab,
    info: &LabInfo,
    key: &TailnetAuthKey,
) -> Result<TailnetStatus> {
    let config = GatewayConfig {
        lab_id: lab.lab_id.clone(),
        network: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{}", lab.lab_id),
        ipv4: get_ipv4_addr(&info.ipv4_network, 3)?.to_string(),
        ipv6: info
            .ipv6_network
            .as_ref()
            .map(|net| get_ipv6_addr(net, 3).map(|ip| ip.to_string()))
            .transpose()?,
        routes: routes(lab),
    };
    let settings = &state.config.tailscale;
    let id = tailscale::create_gateway(&state.docker, &config, settings).await?;
    let mut updated = lab.clone();
    updated.tailscale_container_id = Some(id.clone());
    db::update_lab(&state.db, updated)
        .await
        .context("Unable to persist Tailscale gateway ownership")?;
    tailscale::enroll_gateway(&state.docker, &id, &config, key, settings).await
}

fn routes(lab: &DbLab) -> Vec<String> {
    let mut routes = vec![lab.management_network.clone()];
    routes.extend(lab.management_network_v6.iter().cloned());
    routes
}

#[instrument(skip(state, lab), fields(lab_id = %lab.lab_id), level = "debug")]
pub(crate) async fn inspect(state: &AppState, lab: &DbLab) -> Option<TailnetStatus> {
    let id = lab.tailscale_container_id.as_deref()?;
    let routes = routes(lab);
    let result = tailscale::gateway_status(
        &state.docker,
        id,
        &lab.lab_id,
        routes.clone(),
        &state.config.tailscale,
    )
    .await;
    match result {
        Ok(status) => Some(status),
        Err(error) => {
            tracing::warn!(%error, "Unable to inspect Tailscale gateway");
            Some(TailnetStatus::unavailable(routes))
        }
    }
}

#[instrument(skip(state, lab), fields(lab_id = %lab.lab_id))]
pub(crate) async fn resume(state: &AppState, lab: &DbLab) -> Result<Option<TailnetStatus>> {
    let Some(id) = lab.tailscale_container_id.as_deref() else {
        return Ok(None);
    };
    tailscale::resume_gateway(
        &state.docker,
        id,
        &lab.lab_id,
        routes(lab),
        &state.config.tailscale,
    )
    .await
    .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enrollment_preflight_rejects_missing_key_and_plaintext_transport() {
        let manifest: Manifest =
            toml::from_str("name='test'\nnodes=[]\n[tailscale]\nenabled=true\nauth_key_env='KEY'")
                .unwrap();
        let key = TailnetAuthKey::new("tskey-auth-fixture".into());
        assert!(validate_enrollment(&manifest, None, true).is_err());
        assert!(validate_enrollment(&manifest, Some(&key), false).is_err());
        assert!(validate_enrollment(&manifest, Some(&key), true).unwrap());
        let disabled = Manifest::default();
        assert!(!validate_enrollment(&disabled, None, false).unwrap());
        assert!(validate_enrollment(&disabled, Some(&key), true).is_err());
    }
}
