//! After readiness: the lab's Kubernetes clusters, Keycloak realms and Vaults.

use std::net::Ipv4Addr;
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
            errors.push(service_error(format!(
                "Kubernetes cluster {name} did not become ready"
            )));
        }
        clusters
    };

    // Make the Sherpa realm on Keycloak nodes, then initialise and unseal Vault nodes
    // running the generated config and let the clusters that use them log in.
    let mut keycloaks = Vec::new();
    let mut vaults = Vec::new();
    for node in inputs.manifest_nodes {
        let initialises_vault =
            vault::sherpa_initialises_vault(node.model, node.ztp.config.is_some());
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
        if !initialises_vault {
            match keycloak::ensure_keycloak_realm(
                lab_id,
                &node.name,
                node_address,
                &inputs.lab_ca.pair.cert,
                inputs.owner,
                inputs.timeout,
                progress,
            )
            .await
            {
                Ok(info) => keycloaks.push(info),
                Err(error) => errors.push(service_error(format!("{error:#}"))),
            }
            continue;
        }
        if let Some(info) = unseal_vault(build, inputs, &node.name, node_address, &mut errors).await
        {
            vaults.push(info);
        }
    }

    Ok(LabServices {
        clusters,
        keycloaks,
        vaults,
        errors,
    })
}

/// Initialise and unseal a Vault node, and let the clusters that use it log in. `None`, with
/// the error recorded, when the Vault does not come up; a cluster that cannot be given access
/// is recorded too, and the Vault is still returned.
#[instrument(skip_all, fields(lab_id = %build.lab_id, %node_name), level = "debug")]
async fn unseal_vault(
    build: &LabBuild<'_>,
    inputs: &ServiceInputs<'_>,
    node_name: &str,
    node_address: Ipv4Addr,
    errors: &mut Vec<data::UpError>,
) -> Option<data::VaultInfo> {
    let info = match vault::ensure_vault_unsealed(
        build.lab_id,
        node_name,
        node_address,
        &inputs.lab_ca.pair.cert,
        inputs.timeout,
        build.progress,
    )
    .await
    {
        Ok(info) => info,
        Err(error) => {
            errors.push(service_error(format!("{error:#}")));
            return None;
        }
    };
    if let Err(error) = talos::grant_vault_access(
        inputs.lab_clusters,
        node_name,
        node_address,
        &inputs.lab_ca.pair.cert,
        &info.root_token,
    )
    .await
    {
        errors.push(service_error(format!("{error:#}")));
    }
    Some(info)
}

/// A lab service that did not come up: a critical error of the readiness phase. Nothing sets it
/// up again, so the lab is only partly created.
fn service_error(message: String) -> data::UpError {
    data::UpError {
        phase: "NodeReadiness".to_string(),
        message,
        is_critical: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lab_service_that_did_not_come_up_fails_the_up() {
        // M6.1: a cluster that did not bootstrap, or a Vault or Keycloak that failed to set
        // up, is not retried, so the lab is only partly created.
        assert!(
            service_error("Kubernetes cluster k1 did not become ready".to_string()).is_critical
        );
    }
}
