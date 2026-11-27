//! Tailnet orchestration; callers authenticate and authorize the lab first.
use anyhow::{Context, Result};
use bollard::Docker;
use container::tailscale::{self, GatewayConfig};
use shared::data::{DbLab, LabInfo, TailnetAuthKey, TailnetState, TailnetStatus};
use shared::konst::TAILSCALE_GATEWAY_HOST_OFFSET;
use shared::util::{get_ipv4_addr, get_ipv6_addr};
use topology::Manifest;
use tracing::instrument;

use crate::daemon::state::AppState;
use crate::konst::SHERPA_MANAGEMENT_NETWORK_NAME;
use crate::services::error::ServiceError;
use shared::validators::validate_tailscale_key;
use topology::validate::validate_tailscale;

#[instrument(skip(manifest, key), level = "debug")]
pub(crate) fn validate_enrollment(
    manifest: &Manifest,
    key: Option<&TailnetAuthKey>,
    tls_enabled: bool,
) -> Result<bool> {
    validate_tailscale(manifest).map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    let enabled = manifest.tailscale.as_ref().is_some_and(|c| c.enabled);
    if enabled {
        if !tls_enabled {
            return Err(ServiceError::Invalid(
                "Tailscale enrollment requires server TLS".to_owned(),
            )
            .into());
        }
        let key = key.ok_or_else(|| ServiceError::Invalid("Tailscale is enabled but no auth key was supplied; use the CLI with tailscale.auth_key_env".to_owned()))?;
        validate_tailscale_key(key.expose())
            .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    } else if key.is_some() {
        return Err(ServiceError::Invalid(
            "Tailscale credential supplied without enabling tailscale in the manifest".to_owned(),
        )
        .into());
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
        ipv4: get_ipv4_addr(&info.ipv4_network, TAILSCALE_GATEWAY_HOST_OFFSET)?.to_string(),
        ipv6: info
            .ipv6_network
            .as_ref()
            .map(|net| get_ipv6_addr(net, TAILSCALE_GATEWAY_HOST_OFFSET).map(|ip| ip.to_string()))
            .transpose()?,
        routes: routes(lab),
    };
    let settings = &state.config.tailscale;
    let id = tailscale::create_gateway(&state.docker, &config, settings).await?;
    let lab_record = lab.id.as_ref().context("Lab record has no id")?;
    db::update_lab_tailscale_container(&state.db, lab_record, Some(&id))
        .await
        .context("Unable to persist Tailscale gateway ownership")?;
    tailscale::enroll_gateway(
        &state.docker,
        &id,
        &config.lab_id,
        &config.routes,
        key,
        settings,
    )
    .await
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

/// Refuse to resume `lab` when the gateway container its record names is gone: a key cannot
/// enroll it, and only a new `up` creates it with a fresh identity.
///
/// # Errors
///
/// Fails when the gateway is gone, or Docker cannot be asked.
#[instrument(skip(docker, lab), fields(lab_id = %lab.lab_id), level = "debug")]
pub(crate) async fn require_gateway(docker: &Docker, lab: &DbLab) -> Result<()> {
    let Some(id) = lab.tailscale_container_id.as_deref() else {
        return Ok(());
    };
    match container::container_state(docker, id)
        .await
        .context("Failed to check the lab's Tailscale gateway")?
    {
        Some(_) => Ok(()),
        None => Err(ServiceError::TailscaleGatewayMissing {
            container: tailscale::gateway_name(&lab.lab_id),
        }
        .into()),
    }
}

/// The key a resume enrolls the lab's gateway with, from the gateway's live `gateway` state.
#[instrument(skip(key), level = "debug")]
pub(crate) fn resume_credential<'a>(
    gateway: Option<&TailnetState>,
    auth_key_env: Option<&str>,
    whole_lab: bool,
    key: Option<&'a TailnetAuthKey>,
    tls_enabled: bool,
) -> Result<Option<&'a TailnetAuthKey>> {
    let Some(gateway) = gateway else {
        if key.is_some() {
            return Err(ServiceError::Invalid(
                "Tailscale credential supplied for a lab without a Tailscale gateway".to_owned(),
            )
            .into());
        }
        return Ok(None);
    };
    if !whole_lab {
        if key.is_some() {
            return Err(ServiceError::Invalid(
                "Tailscale credential supplied for a single-node resume, which does not reconnect the gateway"
                    .to_owned(),
            )
            .into());
        }
        return Ok(None);
    }
    if *gateway == TailnetState::Connected {
        return Ok(None);
    }
    if !tls_enabled {
        return Err(
            ServiceError::Invalid("Tailscale enrollment requires server TLS".to_owned()).into(),
        );
    }
    let key = key.ok_or_else(|| ServiceError::ResumeNeedsAuthKey {
        auth_key_env: auth_key_env.map(str::to_owned),
    })?;
    validate_tailscale_key(key.expose())
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    Ok(Some(key))
}

/// Enroll the gateway as a new device, stopping it again when the enrollment fails.
#[instrument(skip(state, lab, key), fields(lab_id = %lab.lab_id))]
pub(crate) async fn resume(
    state: &AppState,
    lab: &DbLab,
    key: &TailnetAuthKey,
) -> Result<Option<TailnetStatus>> {
    let Some(id) = lab.tailscale_container_id.as_deref() else {
        return Ok(None);
    };
    tailscale::resume_gateway(
        &state.docker,
        id,
        &lab.lab_id,
        &routes(lab),
        key,
        &state.config.tailscale,
    )
    .await
    .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::docker_mock::{DockerMock, Fault};
    use serde_json::json;
    use shared::data::{LabPhase, RecordId};

    /// A lab `test1234` whose record names the gateway container `gateway-id`.
    fn lab_with_gateway() -> DbLab {
        DbLab {
            id: Some(RecordId::new("lab", "l1")),
            lab_id: "test1234".to_string(),
            name: "core".to_string(),
            user: RecordId::new("user", "alice"),
            management_network: "172.31.1.0/24".to_string(),
            gateway_ipv4: "172.31.1.1".to_string(),
            router_ipv4: "172.31.1.2".to_string(),
            tailscale_container_id: Some("gateway-id".to_string()),
            management_network_v6: None,
            gateway_ipv6: None,
            router_ipv6: None,
            loopback_network_v6: None,
            phase: LabPhase::Ready,
            client_host: None,
            client_folder: None,
            description: None,
        }
    }

    #[tokio::test]
    async fn tailnet_suspend_027_a_whole_lab_resume_of_a_lab_whose_gateway_is_gone_is_refused_saying_so()
     {
        let gone = DockerMock::new(json!([]), None).await;
        let refused = require_gateway(&gone.docker, &lab_with_gateway())
            .await
            .unwrap_err();
        assert_eq!(
            refused.downcast_ref::<ServiceError>(),
            Some(&ServiceError::TailscaleGatewayMissing {
                container: "sherpa-tailnet-test1234".to_string()
            })
        );
        let stopped = DockerMock::new(json!([]), Some(Fault::Gateway)).await;
        require_gateway(&stopped.docker, &lab_with_gateway())
            .await
            .unwrap();
    }

    const TAILNET_MANIFEST: &str =
        "name='test'\nnodes=[]\n[tailscale]\nenabled=true\nauth_key_env='KEY'";

    #[test]
    fn tailnet_key_007_an_enabled_up_with_no_key_or_tls_off_is_refused() {
        let manifest: Manifest = toml::from_str(TAILNET_MANIFEST).unwrap();
        let key = TailnetAuthKey::new("tskey-auth-fixture".into());
        assert!(validate_enrollment(&manifest, None, true).is_err());
        assert!(validate_enrollment(&manifest, Some(&key), false).is_err());
        assert!(validate_enrollment(&manifest, Some(&key), true).unwrap());
    }

    #[test]
    fn tailnet_key_008_a_key_for_a_lab_without_tailscale_enabled_is_refused() {
        let disabled: Manifest =
            toml::from_str("name='test'\nnodes=[]\n[tailscale]\nenabled=false\nauth_key_env='KEY'")
                .unwrap();
        let key = TailnetAuthKey::new("tskey-auth-fixture".into());
        for manifest in [Manifest::default(), disabled] {
            assert!(validate_enrollment(&manifest, Some(&key), true).is_err());
        }
    }

    #[test]
    fn tailnet_suspend_006_a_resume_of_a_gateway_not_connected_with_no_key_a_malformed_key_or_tls_off_is_refused()
     {
        let key = TailnetAuthKey::new("tskey-auth-fixture".into());
        let invalid = TailnetAuthKey::new("not-a-key".into());
        for state in [
            TailnetState::Stopped,
            TailnetState::NeedsAuthentication,
            TailnetState::Unavailable,
        ] {
            let refused =
                resume_credential(Some(&state), Some("LAB_KEY"), true, None, true).unwrap_err();
            assert_eq!(
                refused.downcast_ref::<ServiceError>(),
                Some(&ServiceError::ResumeNeedsAuthKey {
                    auth_key_env: Some("LAB_KEY".to_owned())
                })
            );
            assert!(resume_credential(Some(&state), None, true, Some(&key), false).is_err());
            assert!(resume_credential(Some(&state), None, true, Some(&invalid), true).is_err());
            assert!(
                resume_credential(Some(&state), None, true, Some(&key), true)
                    .unwrap()
                    .is_some()
            );
        }
    }

    #[test]
    fn tailnet_suspend_007_a_key_for_a_lab_with_no_gateway_or_a_single_node_resume_is_refused() {
        let key = TailnetAuthKey::new("tskey-auth-fixture".into());
        assert!(
            resume_credential(None, None, true, None, false)
                .unwrap()
                .is_none()
        );
        assert!(resume_credential(None, None, true, Some(&key), true).is_err());
        for state in [TailnetState::Connected, TailnetState::Stopped] {
            assert!(
                resume_credential(Some(&state), None, false, None, false)
                    .unwrap()
                    .is_none()
            );
            assert!(resume_credential(Some(&state), None, false, Some(&key), true).is_err());
        }
    }

    #[test]
    fn tailnet_suspend_021_a_whole_lab_resume_of_a_connected_gateway_needs_no_key_and_ignores_one_sent()
     {
        let connected = TailnetState::Connected;
        let key = TailnetAuthKey::new("tskey-auth-fixture".into());
        let invalid = TailnetAuthKey::new("not-a-key".into());
        assert!(
            resume_credential(Some(&connected), None, true, None, true)
                .unwrap()
                .is_none()
        );
        for sent in [&key, &invalid] {
            for tls_enabled in [true, false] {
                assert!(
                    resume_credential(Some(&connected), None, true, Some(sent), tls_enabled)
                        .unwrap()
                        .is_none()
                );
            }
        }
    }
}
