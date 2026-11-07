//! After readiness: the lab's Kubernetes clusters, Keycloak realms and Vaults.

use std::time::Duration;

use anyhow::Result;
use tracing::instrument;

use super::context::LabBuild;
use crate::services::keycloak;
use crate::services::lab_operations::LabOperation;
use crate::services::talos;
use crate::services::talos::LabCluster;
use crate::services::vault;
use crate::tls::lab_ca::LabCa;

use shared::data;

/// What starting the lab's services needs.
pub(super) struct ServiceInputs<'a> {
    pub(super) manifest_nodes: &'a [topology::Node],
    pub(super) container_nodes: &'a [topology::NodeExpanded],
    pub(super) lab_clusters: &'a [LabCluster],
    pub(super) lab_ca: &'a LabCa,
    /// The lab owner, given an account in each Keycloak realm.
    pub(super) owner: &'a str,
    pub(super) timeout: Duration,
}

/// The lab's Kubernetes clusters, Keycloak realms and Vaults, and what did not come up.
pub(super) struct LabServices {
    pub(super) clusters: Vec<data::ClusterInfo>,
    pub(super) keycloaks: Vec<data::KeycloakInfo>,
    pub(super) vaults: Vec<data::VaultInfo>,
    pub(super) errors: Vec<data::UpError>,
}

/// Once the nodes answer: bootstrap the Kubernetes clusters, make the Sherpa realm on Keycloak
/// nodes, and initialise and unseal Vault nodes running the generated config, letting the
/// clusters that use them log in. A service that does not come up is a non-critical error.
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn start_lab_services(
    build: &LabBuild<'_>,
    operation: &LabOperation,
    inputs: &ServiceInputs<'_>,
) -> Result<LabServices> {
    let LabBuild {
        lab_id, progress, ..
    } = *build;
    let mut errors = Vec::new();
    // Bootstrap Kubernetes clusters once their nodes answer.
    let clusters = if inputs.lab_clusters.is_empty() {
        Vec::new()
    } else {
        let (clusters, not_ready) = talos::bring_up_clusters(
            lab_id,
            inputs.lab_clusters,
            &inputs.lab_ca.pair.cert,
            inputs.timeout,
            progress,
            operation,
        )
        .await?;
        for name in not_ready {
            errors.push(data::UpError {
                phase: "NodeReadiness".to_string(),
                message: format!("Kubernetes cluster {name} did not become ready"),
                is_critical: false,
            });
        }
        clusters
    };

    // Make the Sherpa realm on Keycloak nodes, then initialise and unseal Vault nodes
    // running the generated config and let the clusters that use them log in.
    let mut keycloaks = Vec::new();
    let mut vaults = Vec::new();
    for node in inputs.manifest_nodes {
        let initialises_vault =
            vault::sherpa_initialises_vault(&node.model, node.ztp_config.is_some());
        if node.model != data::NodeModel::KeycloakServer && !initialises_vault {
            continue;
        }
        let Some(node_address) = inputs
            .container_nodes
            .iter()
            .find(|container| container.name == node.name)
            .and_then(|container| container.ipv4_address)
        else {
            continue;
        };
        let timeout = inputs.timeout;
        if !initialises_vault {
            match keycloak::ensure_keycloak_realm(
                lab_id,
                &node.name,
                node_address,
                &inputs.lab_ca.pair.cert,
                inputs.owner,
                timeout,
                progress,
            )
            .await
            {
                Ok(info) => keycloaks.push(info),
                Err(error) => errors.push(data::UpError {
                    phase: "NodeReadiness".to_string(),
                    message: format!("{error:#}"),
                    is_critical: false,
                }),
            }
            continue;
        }
        let info = match vault::ensure_vault_unsealed(
            lab_id,
            &node.name,
            node_address,
            &inputs.lab_ca.pair.cert,
            timeout,
            progress,
        )
        .await
        {
            Ok(info) => info,
            Err(error) => {
                errors.push(data::UpError {
                    phase: "NodeReadiness".to_string(),
                    message: format!("{error:#}"),
                    is_critical: false,
                });
                continue;
            }
        };
        if let Err(error) = talos::grant_vault_access(
            inputs.lab_clusters,
            &node.name,
            node_address,
            &inputs.lab_ca.pair.cert,
            &info.root_token,
        )
        .await
        {
            errors.push(data::UpError {
                phase: "NodeReadiness".to_string(),
                message: format!("{error:#}"),
                is_critical: false,
            });
        }
        vaults.push(info);
    }

    Ok(LabServices {
        clusters,
        keycloaks,
        vaults,
        errors,
    })
}
