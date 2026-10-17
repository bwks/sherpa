use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use axum::http::Response;
use http_body_util::BodyExt;
use serde_json::Value;
use tracing::instrument;

use shared::data::StatusKind;
use shared::konst::{READINESS_SLEEP, TALOS_API_PORT};
use template::TalosClusterSecrets;

use super::plan::ClusterPlan;
use crate::services::progress::ProgressSender;
use crate::tls::generator::PemPair;

/// gRPC path of the Talos bootstrap call.
const BOOTSTRAP_PATH: &str = "/machine.MachineService/Bootstrap";
/// A gRPC message frame holding an empty `BootstrapRequest`: no compression, zero length.
const EMPTY_GRPC_FRAME: [u8; 5] = [0, 0, 0, 0, 0];
/// gRPC status codes Talos returns from Bootstrap.
const GRPC_OK: &str = "0";
const GRPC_ALREADY_EXISTS: &str = "6";
/// How long a single API request may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Result of asking a control plane to bootstrap etcd.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootstrapOutcome {
    Bootstrapped,
    /// The node's etcd already has data: an earlier bootstrap of this node succeeded.
    AlreadyBootstrapped,
}

/// Ask one control plane to bootstrap etcd.
///
/// Talos accepts a second bootstrap while the first is still running, so this must be sent
/// to one control plane per cluster only. Retrying against the same node is safe: once etcd
/// has data, Talos answers `ALREADY_EXISTS`.
#[instrument(skip(secrets, admin), fields(%node))]
pub async fn bootstrap(
    node: Ipv4Addr,
    secrets: &TalosClusterSecrets,
    admin: &PemPair,
) -> Result<BootstrapOutcome> {
    let client = api_client(&secrets.os_ca_cert, admin, true)?;
    let response = client
        .post(format!("https://{node}:{TALOS_API_PORT}{BOOTSTRAP_PATH}"))
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .body(EMPTY_GRPC_FRAME.to_vec())
        .send()
        .await
        .with_context(|| format!("Failed to reach the Talos API on {node}"))?;
    if !response.status().is_success() {
        bail!("Talos API on {node} returned HTTP {}", response.status());
    }

    // A refusal arrives as a trailers-only response (status in the headers); success as a
    // message followed by trailers.
    let header_status = header_value(response.headers(), "grpc-status");
    let header_message = header_value(response.headers(), "grpc-message");
    let collected = Response::from(response)
        .into_body()
        .collect()
        .await
        .with_context(|| format!("Failed to read the bootstrap response from {node}"))?;
    let (trailer_status, trailer_message) = match collected.trailers() {
        Some(trailers) => (
            header_value(trailers, "grpc-status"),
            header_value(trailers, "grpc-message"),
        ),
        None => (None, None),
    };
    bootstrap_outcome(
        header_status.or(trailer_status).as_deref(),
        header_message.or(trailer_message).as_deref(),
    )
    .with_context(|| format!("Talos refused to bootstrap {node}"))
}

/// Interpret the gRPC status of a Bootstrap call.
fn bootstrap_outcome(status: Option<&str>, message: Option<&str>) -> Result<BootstrapOutcome> {
    match status {
        Some(GRPC_OK) => Ok(BootstrapOutcome::Bootstrapped),
        Some(GRPC_ALREADY_EXISTS) => Ok(BootstrapOutcome::AlreadyBootstrapped),
        Some(code) => bail!("gRPC status {code}: {}", message.unwrap_or("no message")),
        None => bail!("response had no gRPC status"),
    }
}

/// Wait until every member of the cluster is a Ready Kubernetes node.
///
/// Returns `false` if the timeout passes first; the caller reports that but does not fail,
/// as with node readiness.
#[instrument(skip(plan, secrets, admin, progress), fields(cluster = %plan.name))]
pub async fn wait_for_kubernetes(
    plan: &ClusterPlan,
    secrets: &TalosClusterSecrets,
    admin: &PemPair,
    timeout: Duration,
    progress: &ProgressSender,
) -> Result<bool> {
    let client = api_client(&secrets.kubernetes_ca_cert, admin, false)?;
    let url = format!("{}/api/v1/nodes", plan.endpoint_url());
    let expected = plan.member_count();
    let started = Instant::now();
    while started.elapsed() < timeout {
        let ready = match client.get(&url).send().await {
            Ok(response) if response.status().is_success() => match response.bytes().await {
                Ok(body) => match serde_json::from_slice::<Value>(&body) {
                    Ok(nodes) => ready_node_count(&nodes),
                    Err(error) => {
                        tracing::debug!(error = %error, "Kubernetes node list was not JSON yet");
                        0
                    }
                },
                Err(error) => {
                    tracing::debug!(error = %error, "Kubernetes node list could not be read");
                    0
                }
            },
            Ok(response) => {
                tracing::debug!(status = %response.status(), "Kubernetes API not ready yet");
                0
            }
            Err(error) => {
                tracing::debug!(error = %error, "Kubernetes API not reachable yet");
                0
            }
        };
        if ready >= expected {
            return Ok(true);
        }
        let _ = progress.send_status(
            format!(
                "Cluster {} - {ready}/{expected} Kubernetes nodes ready",
                plan.name
            ),
            StatusKind::Waiting,
        );
        tokio::time::sleep(Duration::from_secs(READINESS_SLEEP)).await;
    }
    Ok(false)
}

/// Count nodes whose `Ready` condition is `True` in a Kubernetes NodeList.
fn ready_node_count(nodes: &Value) -> usize {
    let Some(items) = nodes["items"].as_array() else {
        return 0;
    };
    items
        .iter()
        .filter(|node| {
            node["status"]["conditions"]
                .as_array()
                .is_some_and(|conditions| {
                    conditions.iter().any(|condition| {
                        condition["type"] == "Ready" && condition["status"] == "True"
                    })
                })
        })
        .count()
}

/// An HTTPS client that trusts only `ca_cert` and presents `identity`.
fn api_client(ca_cert: &str, identity: &PemPair, http2: bool) -> Result<reqwest::Client> {
    let mut identity_pem = identity.cert.clone().into_bytes();
    identity_pem.extend_from_slice(identity.key.as_bytes());
    let builder = reqwest::Client::builder()
        .tls_certs_only([reqwest::Certificate::from_pem(ca_cert.as_bytes())
            .context("Failed to load cluster CA certificate")?])
        .identity(
            reqwest::Identity::from_pem(&identity_pem)
                .context("Failed to load cluster client certificate")?,
        )
        .timeout(REQUEST_TIMEOUT);
    let builder = if http2 {
        builder.http2_prior_knowledge()
    } else {
        builder
    };
    builder
        .build()
        .context("Failed to build cluster API client")
}

fn header_value(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bootstrap_statuses_are_interpreted() {
        assert_eq!(
            bootstrap_outcome(Some("0"), Some("")).unwrap(),
            BootstrapOutcome::Bootstrapped
        );
        assert_eq!(
            bootstrap_outcome(Some("6"), Some("etcd data directory is not empty")).unwrap(),
            BootstrapOutcome::AlreadyBootstrapped
        );
        let error = bootstrap_outcome(Some("14"), Some("unavailable"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("14") && error.contains("unavailable"));
        assert!(bootstrap_outcome(None, None).is_err());
    }

    #[test]
    fn only_nodes_with_a_true_ready_condition_count() {
        let nodes = json!({ "items": [
            { "status": { "conditions": [{ "type": "Ready", "status": "True" }] } },
            { "status": { "conditions": [{ "type": "Ready", "status": "False" }] } },
            { "status": { "conditions": [{ "type": "MemoryPressure", "status": "True" }] } },
            { "status": {} },
        ]});
        assert_eq!(ready_node_count(&nodes), 1);
        assert_eq!(ready_node_count(&json!({})), 0);
    }
}
