//! After readiness: the lab's Kubernetes clusters, Keycloak realms and Vaults.

use std::net::Ipv4Addr;
use std::time::Duration;

use anyhow::{Result, bail};
use tracing::instrument;

use super::context::LabBuild;
use crate::services::error::{ServiceError, caller_message};
use crate::services::keycloak;
use crate::services::lab_operations::LabOperation;
use crate::services::node_lifecycle;
use crate::services::talos;
use crate::services::talos::LabCluster;
use crate::services::vault;
use crate::tls::lab_ca::LabCa;

use shared::data;
use shared::data::OperationEvent;

/// What starting the lab's services needs.
pub(super) struct ServiceInputs<'a> {
    pub(super) manifest_nodes: &'a [topology::Node],
    pub(super) container_nodes: &'a [topology::NodeExpanded],
    pub(super) lab_clusters: &'a [LabCluster],
    pub(super) lab_ca: &'a LabCa,
    pub(super) timeout: Duration,
}

/// The lab's Kubernetes clusters, Keycloak realms and Vaults, the workers of the clusters that
/// became Ready, and what did not come up.
pub(super) struct LabServices {
    pub(super) clusters: Vec<data::ClusterInfo>,
    pub(super) ready_workers: Vec<String>,
    pub(super) keycloaks: Vec<data::KeycloakInfo>,
    pub(super) vaults: Vec<data::VaultInfo>,
    pub(super) errors: Vec<data::UpError>,
}

/// Once the nodes answer: bootstrap the Kubernetes clusters, make the Sherpa realm on Keycloak
/// nodes, and initialise and unseal Vault nodes running the generated config, letting the
/// clusters that use them log in. A service that does not come up is a critical error, and the
/// lab is left partly created.
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
    let mut ready_workers = Vec::new();
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
        for cluster in inputs.lab_clusters {
            if !not_ready.contains(&cluster.plan.name) {
                ready_workers.extend(cluster.plan.workers.iter().map(|w| w.name.clone()));
            }
        }
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
                inputs.timeout,
                progress,
            )
            .await
            {
                Ok(info) => keycloaks.push(info),
                Err(error) => errors.push(service_error(format!(
                    "Keycloak node '{}' was not set up: {}",
                    node.name,
                    caller_message(&error, "Failed to set up a Keycloak realm")
                ))),
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
        ready_workers,
        keycloaks,
        vaults,
        errors,
    })
}

/// Record each worker of a Ready cluster ready, as a worker is ready once its cluster is, and
/// report it in the state it then has in `nodes`. A worker an operation moved meanwhile, such as
/// one stopped, is reported as it is.
///
/// # Errors
///
/// Fails when a worker has no record or the database fails.
#[instrument(skip(db, operation, lab_node_data, nodes), level = "debug")]
pub(super) async fn record_ready_workers(
    db: &db::Database,
    operation: &LabOperation,
    lab_node_data: &[data::LabNodeData],
    ready_workers: &[String],
    nodes: &mut [data::NodeInfo],
) -> Result<()> {
    for worker in ready_workers {
        let Some(record) = lab_node_data.iter().find(|node| node.name == *worker) else {
            bail!("Node {worker} has no record");
        };
        let record_id = db::get_node_id(&record.record)?;
        if let Err(error) =
            node_lifecycle::record(db, operation, &record_id, OperationEvent::Ready).await
            && !matches!(ServiceError::find(&error), Some(ServiceError::Conflict(_)))
        {
            return Err(error);
        }
        let state = db::get_node(db, record_id).await?.state;
        for node in nodes.iter_mut().filter(|node| node.name == *worker) {
            node.status = state;
        }
    }
    Ok(())
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
            errors.push(service_error(format!(
                "Vault node '{node_name}' was not unsealed: {}",
                caller_message(&error, "Failed to unseal a Vault")
            )));
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
        errors.push(service_error(format!(
            "The clusters were not given access to Vault node '{node_name}': {}",
            caller_message(&error, "Failed to give clusters access to a Vault")
        )));
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
    use shared::data::{ModelKind, NodeKind, NodeModel, NodeState};

    use super::*;
    use crate::services::lab_operations::{Operation, begin};
    use crate::services::node_lifecycle::tests::lab_of;

    #[tokio::test]
    async fn cluster_bootstrap_011_the_workers_of_a_ready_cluster_are_reported_running() {
        let (db, _, ids) = lab_of("svc-wk01", 2).await;
        let operation = begin("svc-wk01", Operation::Up).await;
        let mut lab_node_data = Vec::new();
        let mut nodes = Vec::new();
        for (index, id) in (1..).zip(&ids) {
            node_lifecycle::record(&db, &operation, id, OperationEvent::Launched)
                .await
                .unwrap();
            let record = db::get_node(&db, id.clone()).await.unwrap();
            lab_node_data.push(data::LabNodeData {
                name: record.name.clone(),
                model_kind: ModelKind::default_for(NodeModel::TalosLinux),
                index,
                record: record.clone(),
            });
            nodes.push(data::NodeInfo {
                name: record.name,
                kind: NodeKind::VirtualMachine,
                model: NodeModel::TalosLinux,
                status: NodeState::Starting,
                ip_address: None,
                ssh_port: None,
            });
        }

        record_ready_workers(
            &db,
            &operation,
            &lab_node_data,
            &["node1".to_owned()],
            &mut nodes,
        )
        .await
        .unwrap();

        assert_eq!(nodes[0].status, NodeState::Running);
        assert_eq!(
            db::get_node(&db, ids[0].clone()).await.unwrap().state,
            NodeState::Running
        );
        assert_eq!(nodes[1].status, NodeState::Starting);
    }
}
