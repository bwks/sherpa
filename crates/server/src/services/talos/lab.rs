//! Talos clusters in the lab lifecycle: preparing clusters during `up`, building node machine
//! configs, bootstrapping, credentials, and the redeploy and download paths.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tracing::instrument;

use shared::data::{ClusterInfo, ClusterRole, NetworkV4, StatusKind};
use shared::konst::{READINESS_SLEEP, SHERPA_LABS_PATH};
use template::{
    TalosClusterSecrets, TalosNodeConfig, kubeconfig, talos_machine_config, talosconfig,
};

use super::bootstrap::{BootstrapOutcome, bootstrap, wait_for_kubernetes};
use super::plan::{ClusterPlan, MemberInput, plan_clusters};
use super::secrets::{
    CLUSTERS_DIR, KUBECONFIG_FILE, TALOSCONFIG_FILE, client_credentials, cluster_dir,
    generate_cluster_secrets, kubernetes_admin_credentials, load_secrets, save_secrets,
    talos_admin_credentials,
};
use crate::services::progress::ProgressSender;
use crate::tls::generator::write_file_with_permissions;

/// File holding a cluster's plan (endpoint, shared address and members).
const PLAN_FILE: &str = "cluster.toml";

/// A cluster prepared for a lab: its plan and secrets.
#[derive(Clone, Debug)]
pub struct LabCluster {
    pub plan: ClusterPlan,
    pub secrets: TalosClusterSecrets,
}

/// Plan the lab's clusters and generate and save their secrets.
#[instrument(skip(members, management), fields(%lab_id, clusters = cluster_names.len()))]
pub fn prepare_clusters(
    lab_id: &str,
    cluster_names: &[&str],
    members: &[MemberInput],
    management: &NetworkV4,
) -> Result<Vec<LabCluster>> {
    let plans = plan_clusters(cluster_names, members, management)?;
    let mut clusters = Vec::with_capacity(plans.len());
    for plan in plans {
        let secrets = generate_cluster_secrets()
            .with_context(|| format!("Failed to generate secrets for cluster '{}'", plan.name))?;
        let dir = cluster_dir(lab_id, &plan.name);
        save_secrets(&dir, &plan.name, &secrets)?;
        let plan_toml = toml::to_string(&plan).context("Failed to serialize cluster plan")?;
        write_file_with_permissions(&dir.join(PLAN_FILE), plan_toml.as_bytes(), 0o600)
            .with_context(|| format!("Failed to save plan for cluster '{}'", plan.name))?;
        clusters.push(LabCluster { plan, secrets });
    }
    Ok(clusters)
}

/// Load a cluster saved by `prepare_clusters`.
#[instrument(level = "debug", fields(%lab_id, %cluster_name))]
pub fn load_cluster(lab_id: &str, cluster_name: &str) -> Result<LabCluster> {
    let dir = cluster_dir(lab_id, cluster_name);
    let path = dir.join(PLAN_FILE);
    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read the plan of cluster '{cluster_name}'"))?;
    let plan =
        toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))?;
    let secrets = load_secrets(&dir, cluster_name)?;
    Ok(LabCluster { plan, secrets })
}

/// Refuse to redeploy a cluster's control plane.
///
/// A rebuilt control plane would start a fresh etcd member while the old one is still in the
/// cluster's member list; removing that member first is not something Sherpa does yet.
/// Workers can be redeployed: they rejoin with the cluster's saved secrets.
pub fn ensure_redeployable(
    node_name: &str,
    cluster: Option<&str>,
    role: Option<ClusterRole>,
) -> Result<()> {
    if let (Some(cluster), Some(ClusterRole::ControlPlane)) = (cluster, role) {
        bail!(
            "Node '{node_name}' is a control plane of cluster '{cluster}' and cannot be redeployed: its etcd member would have to be removed first. Redeploy workers only, or destroy and recreate the lab"
        );
    }
    Ok(())
}

/// Find a prepared cluster by name.
pub fn find_cluster<'a>(clusters: &'a [LabCluster], name: &str) -> Result<&'a LabCluster> {
    clusters
        .iter()
        .find(|cluster| cluster.plan.name == name)
        .with_context(|| format!("Cluster '{name}' was not prepared"))
}

/// Render one cluster node's machine config.
pub fn node_machine_config(
    cluster: &LabCluster,
    node_name: &str,
    management: &NetworkV4,
) -> Result<String> {
    let plan = &cluster.plan;
    let role = plan.role_of(node_name).with_context(|| {
        format!(
            "Node '{node_name}' is not a member of cluster '{}'",
            plan.name
        )
    })?;
    let endpoint = plan.endpoint_url();
    let management_subnet = management.prefix.to_string();
    talos_machine_config(&TalosNodeConfig {
        cluster_name: &plan.name,
        endpoint: &endpoint,
        hostname: node_name,
        role,
        management_subnet: &management_subnet,
        vip: plan.vip,
        schedulable_control_planes: plan.schedulable_control_planes(),
        secrets: &cluster.secrets,
    })
    .with_context(|| format!("Failed to render the machine config of node '{node_name}'"))
}

/// Bootstrap each cluster once, wait for its nodes, and write the user's credentials.
///
/// Returns the credentials of every cluster, and the names of clusters that did not become
/// ready within `timeout`. A cluster that is not ready is reported, not treated as a failure,
/// the same way node readiness is.
#[instrument(skip(clusters, progress), fields(%lab_id, clusters = clusters.len()))]
pub async fn bring_up_clusters(
    lab_id: &str,
    clusters: &[LabCluster],
    timeout: Duration,
    progress: &ProgressSender,
) -> Result<(Vec<ClusterInfo>, Vec<String>)> {
    let mut infos = Vec::with_capacity(clusters.len());
    let mut not_ready = vec![];
    for cluster in clusters {
        let plan = &cluster.plan;
        // Write credentials first, so the user has them even if the cluster is slow.
        let info = write_credentials(lab_id, cluster)?;
        infos.push(info);

        let ready = match bootstrap_cluster(cluster, timeout, progress).await {
            Some(remaining) => {
                let admin = kubernetes_admin_credentials(&cluster.secrets)?;
                wait_for_kubernetes(plan, &cluster.secrets, &admin, remaining, progress).await?
            }
            None => false,
        };
        if ready {
            let _ = progress.send_status(
                format!(
                    "Cluster {} - Ready ({} Kubernetes nodes)",
                    plan.name,
                    plan.member_count()
                ),
                StatusKind::Done,
            );
        } else {
            tracing::warn!(lab_id = %lab_id, cluster = %plan.name, "Cluster did not become ready");
            let _ = progress.send_status(
                format!(
                    "Cluster {} - Not ready within {} seconds",
                    plan.name,
                    timeout.as_secs()
                ),
                StatusKind::Waiting,
            );
            not_ready.push(plan.name.clone());
        }
    }
    Ok((infos, not_ready))
}

/// Send the bootstrap request to the cluster's first control plane, retrying the same node
/// until it answers. Returns the time left, or `None` if the cluster never bootstrapped.
async fn bootstrap_cluster(
    cluster: &LabCluster,
    timeout: Duration,
    progress: &ProgressSender,
) -> Option<Duration> {
    let plan = &cluster.plan;
    let node = plan.bootstrap_node()?;
    let admin = match talos_admin_credentials(&cluster.secrets) {
        Ok(admin) => admin,
        Err(error) => {
            tracing::error!(cluster = %plan.name, error = ?error, "Failed to issue Talos admin certificate");
            return None;
        }
    };
    let _ = progress.send_status(
        format!(
            "Cluster {} - Bootstrapping etcd on {}",
            plan.name, node.name
        ),
        StatusKind::Progress,
    );
    let started = Instant::now();
    while started.elapsed() < timeout {
        match bootstrap(node.ipv4, &cluster.secrets, &admin).await {
            Ok(BootstrapOutcome::Bootstrapped) => {
                tracing::info!(cluster = %plan.name, node = %node.name, "Cluster bootstrapped");
                return Some(timeout.saturating_sub(started.elapsed()));
            }
            Ok(BootstrapOutcome::AlreadyBootstrapped) => {
                tracing::info!(cluster = %plan.name, node = %node.name, "Cluster was already bootstrapped");
                return Some(timeout.saturating_sub(started.elapsed()));
            }
            Err(error) => {
                tracing::debug!(cluster = %plan.name, error = ?error, "Bootstrap not accepted yet");
                tokio::time::sleep(Duration::from_secs(READINESS_SLEEP)).await;
            }
        }
    }
    None
}

/// Issue admin credentials, write them to the cluster directory and return them.
fn write_credentials(lab_id: &str, cluster: &LabCluster) -> Result<ClusterInfo> {
    let plan = &cluster.plan;
    let secrets = &cluster.secrets;
    let talos_admin = talos_admin_credentials(secrets)?;
    let kube_admin = kubernetes_admin_credentials(secrets)?;
    let endpoints: Vec<String> = plan
        .control_planes
        .iter()
        .map(|member| member.ipv4.to_string())
        .collect();
    let nodes: Vec<String> = endpoints.iter().take(1).cloned().collect();
    let talosconfig = talosconfig(
        &plan.name,
        &endpoints,
        &nodes,
        &client_credentials(&secrets.os_ca_cert, &talos_admin),
    )?;
    let endpoint = plan.endpoint_url();
    let kubeconfig = kubeconfig(
        &plan.name,
        &endpoint,
        &client_credentials(&secrets.kubernetes_ca_cert, &kube_admin),
    )?;
    let dir = cluster_dir(lab_id, &plan.name);
    write_file_with_permissions(&dir.join(TALOSCONFIG_FILE), talosconfig.as_bytes(), 0o600)?;
    write_file_with_permissions(&dir.join(KUBECONFIG_FILE), kubeconfig.as_bytes(), 0o600)?;
    Ok(ClusterInfo {
        name: plan.name.clone(),
        endpoint,
        talosconfig,
        kubeconfig,
    })
}

/// Read the credentials of every cluster in a lab, for `sherpa download`.
#[instrument(level = "debug", fields(%lab_id))]
pub fn read_cluster_credentials(lab_id: &str) -> Result<Vec<ClusterInfo>> {
    let clusters_dir = Path::new(SHERPA_LABS_PATH).join(lab_id).join(CLUSTERS_DIR);
    let entries = match fs::read_dir(&clusters_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(%lab_id, "Lab has no clusters");
            return Ok(vec![]);
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to list {}", clusters_dir.display()));
        }
    };
    let mut infos = vec![];
    for entry in entries {
        let dir = entry
            .with_context(|| format!("Failed to list {}", clusters_dir.display()))?
            .path();
        let Some(name) = dir.file_name().and_then(|name| name.to_str()) else {
            tracing::warn!(path = %dir.display(), "Skipping cluster directory with a non-UTF-8 name");
            continue;
        };
        let cluster = load_cluster(lab_id, name)?;
        infos.push(ClusterInfo {
            name: name.to_owned(),
            endpoint: cluster.plan.endpoint_url(),
            talosconfig: fs::read_to_string(dir.join(TALOSCONFIG_FILE))
                .with_context(|| format!("Failed to read the talosconfig of cluster '{name}'"))?,
            kubeconfig: fs::read_to_string(dir.join(KUBECONFIG_FILE))
                .with_context(|| format!("Failed to read the kubeconfig of cluster '{name}'"))?,
        });
    }
    infos.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(infos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_cluster_control_planes_are_refused_for_redeploy() {
        ensure_redeployable("vm", None, None).unwrap();
        ensure_redeployable("w1", Some("k8s"), Some(ClusterRole::Worker)).unwrap();
        let error = ensure_redeployable("cp1", Some("k8s"), Some(ClusterRole::ControlPlane))
            .unwrap_err()
            .to_string();
        assert!(error.contains("control plane of cluster 'k8s'"));
    }
}
