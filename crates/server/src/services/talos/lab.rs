//! Talos clusters in the lab lifecycle: preparing clusters during `up`, building node machine
//! configs, bootstrapping, credentials, and the redeploy and download paths.

use std::fs;
use std::net::Ipv4Addr;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tracing::instrument;

use db::Database;
use shared::data::{
    ClusterCni, ClusterInfo, ClusterMember, ClusterPlan, ClusterRole, NetworkV4, RecordId,
    StatusKind,
};
use shared::konst::SHERPA_LABS_PATH;
use template::{
    CiliumConfig, ClusterOidcConfig, ClusterVaultConfig, TalosClusterSecrets, TalosNodeConfig,
    kubeconfig, talos_machine_config, talosconfig,
};
use topology::{TalosCluster, find_talos_cluster};

use super::bootstrap::{BootstrapOutcome, bootstrap, shutdown, wait_for_kubernetes};
use super::plan::{MemberInput, plan_clusters};
use super::secrets::{
    KUBECONFIG_FILE, TALOSCONFIG_FILE, client_credentials, cluster_dir, generate_cluster_secrets,
    kubernetes_admin_credentials, load_secrets, save_secrets, service_account_public_key,
    talos_admin_credentials,
};
use crate::konst::{KEYCLOAK_REALM, READINESS_SLEEP};
use crate::services::keycloak::api::keycloak_address;
use crate::services::lab_operations::LabOperation;
use crate::services::progress::ProgressSender;
use crate::services::vault::api::{VaultClient, vault_address};
use crate::services::vault::{ClusterAccess, cluster_auth_mount, configure_cluster_access};
use crate::tls::generator::write_file_with_permissions;

/// A cluster prepared for a lab: its plan and secrets.
#[derive(Clone, Debug)]
pub struct LabCluster {
    pub plan: ClusterPlan,
    pub secrets: TalosClusterSecrets,
}

/// Plan the lab's clusters, generate and save their secrets, and store their plans with the
/// lab `lab_record_id`'s definition, which must already hold the clusters.
///
/// `services` holds the addresses of the Keycloak and Vault nodes clusters use.
#[instrument(skip(db, talos_clusters, members, services, management), fields(%lab_id, clusters = talos_clusters.len()))]
pub async fn prepare_clusters(
    db: &Database,
    lab_record_id: &RecordId,
    lab_id: &str,
    talos_clusters: &[TalosCluster],
    members: &[MemberInput<'_>],
    services: &[ClusterMember],
    management: &NetworkV4,
) -> Result<Vec<LabCluster>> {
    let plans = plan_clusters(talos_clusters, members, services, management)?;
    db::store_talos_cluster_plans(db, lab_record_id, &plans)
        .await
        .context("Failed to store the cluster plans")?;
    let mut clusters = Vec::with_capacity(plans.len());
    for plan in plans {
        let secrets = generate_cluster_secrets()
            .with_context(|| format!("Failed to generate secrets for cluster '{}'", plan.name))?;
        let dir = cluster_dir(Path::new(SHERPA_LABS_PATH), lab_id, &plan.name);
        save_secrets(&dir, &plan.name, &secrets)?;
        clusters.push(LabCluster { plan, secrets });
    }
    Ok(clusters)
}

/// Refuse to redeploy a cluster's control plane.
///
/// A rebuilt control plane would start a fresh etcd member while the old one is still in the
/// cluster's member list; removing that member first is not something Sherpa does yet.
/// Workers can be redeployed: they rejoin with the cluster's saved secrets.
pub fn ensure_redeployable(
    node_name: &str,
    membership: Option<(&TalosCluster, ClusterRole)>,
) -> Result<()> {
    if let Some((cluster, ClusterRole::ControlPlane)) = membership {
        bail!(
            "Node '{node_name}' is a controller of talos cluster '{}' and cannot be redeployed: its etcd member would have to be removed first. Redeploy workers only, or destroy and recreate the lab",
            cluster.name
        );
    }
    Ok(())
}

/// Whether `up` checks this node through Kubernetes instead of its Talos API port.
///
/// A worker's Talos API starts only once it has a certificate from a controller's trustd,
/// which it reaches through the cluster endpoint. In a cluster with three controllers that
/// endpoint is the VIP, which Talos brings up only after etcd is bootstrapped, and the bootstrap
/// waits for node readiness. Waiting on a worker's API port would therefore deadlock, so workers
/// are counted as ready when they are Ready Kubernetes nodes, which `bring_up_clusters` checks.
pub fn readiness_waits_for_kubernetes(clusters: &[TalosCluster], node_name: &str) -> bool {
    matches!(
        find_talos_cluster(clusters, node_name),
        Some((_, ClusterRole::Worker))
    )
}

/// Find a prepared cluster by name.
pub fn find_cluster<'a>(clusters: &'a [LabCluster], name: &str) -> Result<&'a LabCluster> {
    clusters
        .iter()
        .find(|cluster| cluster.plan.name == name)
        .with_context(|| format!("Cluster '{name}' was not prepared"))
}

/// Render one cluster node's machine config. `lab_ca_cert` (PEM) is the CA the cluster's
/// Keycloak and Vault certificates chain to.
pub fn node_machine_config(
    cluster: &LabCluster,
    node_name: &str,
    management: &NetworkV4,
    lab_ca_cert: &str,
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
        cilium: cilium_config(plan)?,
        oidc: oidc_config(plan, lab_ca_cert),
        vault: vault_config(plan, lab_ca_cert),
        secrets: &cluster.secrets,
    })
    .with_context(|| format!("Failed to render the machine config of node '{node_name}'"))
}

/// The login of a cluster that uses a Keycloak node: the node's `sherpa` realm.
fn oidc_config<'a>(plan: &ClusterPlan, lab_ca_cert: &'a str) -> Option<ClusterOidcConfig<'a>> {
    let keycloak = plan.oidc.as_ref()?;
    Some(ClusterOidcConfig {
        issuer_url: format!(
            "{}/realms/{KEYCLOAK_REALM}",
            keycloak_address(keycloak.ipv4)
        ),
        ca_cert: lab_ca_cert,
    })
}

/// The Vault of a cluster that uses a Vault node.
fn vault_config<'a>(plan: &ClusterPlan, lab_ca_cert: &'a str) -> Option<ClusterVaultConfig<'a>> {
    let vault = plan.vault.as_ref()?;
    Some(ClusterVaultConfig {
        address: vault_address(vault.ipv4),
        ca_cert: lab_ca_cert,
        auth_mount: cluster_auth_mount(&plan.name),
    })
}

/// Cilium operator replicas: one per node, up to two.
const CILIUM_MAX_OPERATOR_REPLICAS: usize = 2;

/// A Cilium cluster's template inputs; `None` for Flannel clusters.
fn cilium_config(plan: &ClusterPlan) -> Result<Option<CiliumConfig>> {
    match plan.cni {
        ClusterCni::Flannel => Ok(None),
        ClusterCni::Cilium => {
            let (Some(addresses), Some(gateway), Some(hubble_ui)) = (
                plan.load_balancer_addresses,
                plan.gateway_address(),
                plan.hubble_ui_address(),
            ) else {
                bail!(
                    "Cilium cluster '{}' has no load balancer addresses",
                    plan.name
                );
            };
            Ok(Some(CiliumConfig {
                load_balancer_first: addresses.first,
                load_balancer_last: addresses.last,
                gateway,
                hubble_ui,
                operator_replicas: plan.member_count().min(CILIUM_MAX_OPERATOR_REPLICAS),
            }))
        }
    }
}

/// Bootstrap each cluster once, wait for its nodes, and write the user's credentials.
///
/// Returns the credentials of every cluster, and the names of clusters that did not become
/// ready within `timeout`. A cluster that is not ready is reported, not treated as a failure,
/// the same way node readiness is.
#[instrument(skip(clusters, lab_ca_cert, progress, operation), fields(%lab_id, clusters = clusters.len()))]
pub async fn bring_up_clusters(
    lab_id: &str,
    clusters: &[LabCluster],
    lab_ca_cert: &str,
    timeout: Duration,
    progress: &ProgressSender,
    operation: &LabOperation,
) -> Result<(Vec<ClusterInfo>, Vec<String>)> {
    let mut infos = Vec::with_capacity(clusters.len());
    let mut not_ready = vec![];
    for cluster in clusters {
        // Never write credentials for a lab that is being destroyed.
        operation.ensure_active()?;
        let plan = &cluster.plan;
        // Write credentials first, so the user has them even if the cluster is slow.
        let info = write_credentials(lab_id, cluster, lab_ca_cert)?;
        infos.push(info);

        let ready = match bootstrap_cluster(cluster, timeout, progress, operation).await? {
            Some(remaining) => {
                let admin = kubernetes_admin_credentials(&cluster.secrets)?;
                wait_for_kubernetes(
                    plan,
                    &cluster.secrets,
                    &admin,
                    remaining,
                    progress,
                    operation,
                )
                .await?
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
/// Fails if the lab is destroyed meanwhile.
async fn bootstrap_cluster(
    cluster: &LabCluster,
    timeout: Duration,
    progress: &ProgressSender,
    operation: &LabOperation,
) -> Result<Option<Duration>> {
    let plan = &cluster.plan;
    let Some(node) = plan.bootstrap_node() else {
        return Ok(None);
    };
    let admin = match talos_admin_credentials(&cluster.secrets) {
        Ok(admin) => admin,
        Err(error) => {
            tracing::error!(cluster = %plan.name, error = ?error, "Failed to issue Talos admin certificate");
            return Ok(None);
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
                return Ok(Some(timeout.saturating_sub(started.elapsed())));
            }
            Ok(BootstrapOutcome::AlreadyBootstrapped) => {
                tracing::info!(cluster = %plan.name, node = %node.name, "Cluster was already bootstrapped");
                return Ok(Some(timeout.saturating_sub(started.elapsed())));
            }
            Err(error) => {
                tracing::debug!(cluster = %plan.name, error = ?error, "Bootstrap not accepted yet");
                operation
                    .sleep(Duration::from_secs(READINESS_SLEEP))
                    .await?;
            }
        }
    }
    Ok(None)
}

/// Issue admin credentials, write them to the cluster directory and return them.
fn write_credentials(lab_id: &str, cluster: &LabCluster, lab_ca_cert: &str) -> Result<ClusterInfo> {
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
        oidc_config(plan, lab_ca_cert).as_ref(),
    )?;
    let dir = cluster_dir(Path::new(SHERPA_LABS_PATH), lab_id, &plan.name);
    write_file_with_permissions(&dir.join(TALOSCONFIG_FILE), talosconfig.as_bytes(), 0o600)?;
    write_file_with_permissions(&dir.join(KUBECONFIG_FILE), kubeconfig.as_bytes(), 0o600)?;
    Ok(ClusterInfo {
        name: plan.name.clone(),
        endpoint,
        talosconfig,
        kubeconfig,
        gateway: plan.gateway_address().map(|address| address.to_string()),
        hubble_ui: plan.hubble_ui_address().map(|address| address.to_string()),
    })
}

/// Read the credentials of every cluster of the lab `lab_record_id`, for `sherpa download`:
/// the endpoint and load balancer addresses from its stored plan, and the talosconfig and
/// kubeconfig from its folder in the lab `lab_id`'s folder under `labs`.
#[instrument(level = "debug", skip(db), fields(%lab_id))]
pub async fn read_cluster_credentials(
    db: &Database,
    labs: &Path,
    lab_record_id: &RecordId,
    lab_id: &str,
) -> Result<Vec<ClusterInfo>> {
    let plans = db::list_talos_cluster_plans(db, lab_record_id)
        .await
        .context("Failed to read the lab's cluster plans")?;
    let mut infos = Vec::with_capacity(plans.len());
    for plan in &plans {
        let name = &plan.name;
        let dir = cluster_dir(labs, lab_id, name);
        infos.push(ClusterInfo {
            name: name.clone(),
            endpoint: plan.endpoint_url(),
            gateway: plan.gateway_address().map(|address| address.to_string()),
            hubble_ui: plan.hubble_ui_address().map(|address| address.to_string()),
            talosconfig: fs::read_to_string(dir.join(TALOSCONFIG_FILE))
                .with_context(|| format!("Failed to read the talosconfig of cluster '{name}'"))?,
            kubeconfig: fs::read_to_string(dir.join(KUBECONFIG_FILE))
                .with_context(|| format!("Failed to read the kubeconfig of cluster '{name}'"))?,
        });
    }
    infos.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(infos)
}

/// The cluster `node_name` belongs to, with its role and management address.
pub fn find_member_cluster<'a>(
    clusters: &'a [LabCluster],
    node_name: &str,
) -> Option<(&'a LabCluster, ClusterRole, Ipv4Addr)> {
    clusters.iter().find_map(|cluster| {
        cluster
            .plan
            .member(node_name)
            .map(|(role, member)| (cluster, role, member.ipv4))
    })
}

/// Ask a cluster node to shut down gracefully through its Talos API, with the cluster's
/// saved admin credentials.
#[instrument(skip(cluster), fields(cluster = %cluster.plan.name, %node))]
pub async fn request_shutdown(cluster: &LabCluster, node: Ipv4Addr) -> Result<()> {
    let admin = talos_admin_credentials(&cluster.secrets)?;
    shutdown(node, &cluster.secrets, &admin).await
}

/// Every cluster of the lab `lab_record_id`: its plan from the database and its secrets from
/// its folder in the lab `lab_id`'s folder under `labs`.
#[instrument(level = "debug", skip(db), fields(%lab_id))]
pub async fn load_clusters(
    db: &Database,
    labs: &Path,
    lab_record_id: &RecordId,
    lab_id: &str,
) -> Result<Vec<LabCluster>> {
    let plans = db::list_talos_cluster_plans(db, lab_record_id)
        .await
        .context("Failed to read the lab's cluster plans")?;
    let mut clusters = Vec::with_capacity(plans.len());
    for plan in plans {
        let secrets = load_secrets(&cluster_dir(labs, lab_id, &plan.name), &plan.name)?;
        clusters.push(LabCluster { plan, secrets });
    }
    Ok(clusters)
}

/// Let the workloads of every cluster that uses Vault node `vault_node`, at `node_address`,
/// log in to it and read their secrets. `root_token` is the node's root token.
#[instrument(skip(clusters, lab_ca_pem, root_token), fields(%vault_node, %node_address))]
pub async fn grant_vault_access(
    clusters: &[LabCluster],
    vault_node: &str,
    node_address: Ipv4Addr,
    lab_ca_pem: &str,
    root_token: &str,
) -> Result<()> {
    let client = VaultClient::new(node_address, lab_ca_pem)?;
    for cluster in clusters {
        let plan = &cluster.plan;
        if plan
            .vault
            .as_ref()
            .is_none_or(|vault| vault.name != vault_node)
        {
            continue;
        }
        let public_key = service_account_public_key(&cluster.secrets)?;
        let issuer = plan.endpoint_url();
        configure_cluster_access(
            &client,
            root_token,
            &ClusterAccess {
                cluster_name: &plan.name,
                issuer: &issuer,
                service_account_public_key: &public_key,
            },
        )
        .await
        .with_context(|| {
            format!(
                "Failed to let cluster '{}' log in to Vault node '{vault_node}'",
                plan.name
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use shared::data::{AddressRange, DbLab, NodeModel};
    use topology::{Manifest, Node};

    use crate::services::lab_definition::tests::lab_with;

    use super::*;

    /// A plan member at `172.31.1.<host>`.
    fn member(name: &str, host: u8) -> ClusterMember {
        ClusterMember {
            name: name.to_owned(),
            ipv4: Ipv4Addr::new(172, 31, 1, host),
        }
    }

    /// The plans `up` works out for the lab of `cluster_lab`: an HA Cilium cluster `ha` and a
    /// single-node Flannel cluster `solo`.
    pub(crate) fn cluster_plans() -> Vec<ClusterPlan> {
        vec![
            ClusterPlan {
                name: "ha".to_owned(),
                endpoint: Ipv4Addr::new(172, 31, 1, 11),
                vip: Some(Ipv4Addr::new(172, 31, 1, 11)),
                cni: ClusterCni::Cilium,
                load_balancer_addresses: Some(AddressRange {
                    first: Ipv4Addr::new(172, 31, 1, 13),
                    last: Ipv4Addr::new(172, 31, 1, 16),
                }),
                control_planes: vec![member("cp1", 21), member("cp2", 22), member("cp3", 23)],
                workers: vec![member("w1", 24)],
                oidc: None,
                vault: None,
            },
            ClusterPlan {
                name: "solo".to_owned(),
                endpoint: Ipv4Addr::new(172, 31, 1, 25),
                vip: None,
                cni: ClusterCni::Flannel,
                load_balancer_addresses: None,
                control_planes: vec![member("solo", 25)],
                workers: vec![],
                oidc: None,
                vault: None,
            },
        ]
    }

    /// A keyed database holding the lab `lab-0001` with the Talos nodes of `cluster_plans` at
    /// their addresses, and a definition with its two clusters. No plan is stored.
    pub(crate) async fn cluster_lab() -> (Database, DbLab) {
        let plans = cluster_plans();
        let members: Vec<&ClusterMember> = plans
            .iter()
            .flat_map(|plan| plan.control_planes.iter().chain(&plan.workers))
            .collect();
        let nodes: Vec<(&str, NodeModel)> = members
            .iter()
            .map(|member| (member.name.as_str(), NodeModel::TalosLinux))
            .collect();
        let (db, lab) = lab_with(&nodes).await;
        for record in db::list_nodes_by_lab(&db, lab.id.clone().unwrap())
            .await
            .unwrap()
        {
            let member = members
                .iter()
                .find(|member| member.name == record.name)
                .unwrap();
            db::update_node_mgmt_ipv4(&db, record.id.unwrap(), &member.ipv4.to_string())
                .await
                .unwrap();
        }
        let manifest = Manifest {
            name: "core".to_owned(),
            talos_clusters: Some(
                plans
                    .iter()
                    .map(|plan| TalosCluster {
                        name: plan.name.clone(),
                        controllers: plan.control_planes.iter().map(|m| m.name.clone()).collect(),
                        workers: plan.workers.iter().map(|m| m.name.clone()).collect(),
                        cni: plan.cni,
                        oidc: None,
                        vault: None,
                    })
                    .collect(),
            ),
            nodes: nodes
                .iter()
                .map(|(name, model)| Node {
                    name: (*name).to_owned(),
                    model: *model,
                    ..Node::default()
                })
                .collect(),
            ..Manifest::default()
        };
        db::store_lab_definition(&db, lab.id.as_ref().unwrap(), &manifest)
            .await
            .unwrap();
        (db, lab)
    }

    /// Save secrets and user credentials for each of `plans` in the folder of lab `lab_id`
    /// under `labs`, as `up` does.
    pub(crate) fn save_cluster_files(labs: &Path, lab_id: &str, plans: &[ClusterPlan]) {
        for plan in plans {
            let dir = cluster_dir(labs, lab_id, &plan.name);
            fs::create_dir_all(&dir).unwrap();
            save_secrets(&dir, &plan.name, &generate_cluster_secrets().unwrap()).unwrap();
            fs::write(
                dir.join(TALOSCONFIG_FILE),
                format!("talosconfig of {}", plan.name),
            )
            .unwrap();
            fs::write(
                dir.join(KUBECONFIG_FILE),
                format!("kubeconfig of {}", plan.name),
            )
            .unwrap();
        }
    }

    /// The lab of `cluster_lab` with its plans stored and its cluster files saved under a
    /// temporary labs folder.
    async fn stored_cluster_lab() -> (Database, DbLab, tempfile::TempDir) {
        let (db, lab) = cluster_lab().await;
        db::store_talos_cluster_plans(&db, lab.id.as_ref().unwrap(), &cluster_plans())
            .await
            .unwrap();
        let labs = tempfile::tempdir().unwrap();
        save_cluster_files(labs.path(), &lab.lab_id, &cluster_plans());
        (db, lab, labs)
    }

    #[tokio::test]
    async fn clusters_load_from_their_stored_plans_and_saved_secrets() {
        let (db, lab, labs) = stored_cluster_lab().await;

        let clusters = load_clusters(&db, labs.path(), lab.id.as_ref().unwrap(), &lab.lab_id)
            .await
            .unwrap();

        let plans: Vec<ClusterPlan> = clusters.iter().map(|c| c.plan.clone()).collect();
        assert_eq!(plans, cluster_plans());
        let solo = find_cluster(&clusters, "solo").unwrap();
        assert_eq!(
            solo.secrets,
            load_secrets(&cluster_dir(labs.path(), &lab.lab_id, "solo"), "solo").unwrap()
        );
    }

    #[tokio::test]
    async fn a_lab_without_stored_plans_has_no_clusters() {
        let (db, lab) = cluster_lab().await;
        let labs = tempfile::tempdir().unwrap();
        save_cluster_files(labs.path(), &lab.lab_id, &cluster_plans());

        let clusters = load_clusters(&db, labs.path(), lab.id.as_ref().unwrap(), &lab.lab_id)
            .await
            .unwrap();

        assert!(clusters.is_empty());
    }

    #[tokio::test]
    async fn the_download_credentials_come_from_the_stored_plans_and_saved_files() {
        let (db, lab, labs) = stored_cluster_lab().await;

        let infos =
            read_cluster_credentials(&db, labs.path(), lab.id.as_ref().unwrap(), &lab.lab_id)
                .await
                .unwrap();

        assert_eq!(
            infos,
            [
                ClusterInfo {
                    name: "ha".to_owned(),
                    endpoint: "https://172.31.1.11:6443".to_owned(),
                    talosconfig: "talosconfig of ha".to_owned(),
                    kubeconfig: "kubeconfig of ha".to_owned(),
                    gateway: Some("172.31.1.13".to_owned()),
                    hubble_ui: Some("172.31.1.14".to_owned()),
                },
                ClusterInfo {
                    name: "solo".to_owned(),
                    endpoint: "https://172.31.1.25:6443".to_owned(),
                    talosconfig: "talosconfig of solo".to_owned(),
                    kubeconfig: "kubeconfig of solo".to_owned(),
                    gateway: None,
                    hubble_ui: None,
                },
            ]
        );
    }

    #[tokio::test]
    async fn members_are_found_in_the_loaded_clusters() {
        let (db, lab, labs) = stored_cluster_lab().await;
        let clusters = load_clusters(&db, labs.path(), lab.id.as_ref().unwrap(), &lab.lab_id)
            .await
            .unwrap();

        let (cluster, role, address) = find_member_cluster(&clusters, "w1").unwrap();
        assert_eq!(
            (cluster.plan.name.as_str(), role, address),
            ("ha", ClusterRole::Worker, Ipv4Addr::new(172, 31, 1, 24))
        );
        let (cluster, role, _) = find_member_cluster(&clusters, "solo").unwrap();
        assert_eq!(
            (cluster.plan.name.as_str(), role),
            ("solo", ClusterRole::ControlPlane)
        );
        assert!(find_member_cluster(&clusters, "vm1").is_none());
    }

    #[test]
    fn only_cluster_controllers_are_refused_for_redeploy() {
        let cluster = TalosCluster {
            name: "k8s".to_owned(),
            controllers: vec!["cp1".to_owned()],
            workers: vec!["w1".to_owned()],
            ..TalosCluster::default()
        };
        ensure_redeployable("vm", None).unwrap();
        ensure_redeployable("w1", Some((&cluster, ClusterRole::Worker))).unwrap();
        let error = ensure_redeployable("cp1", Some((&cluster, ClusterRole::ControlPlane)))
            .unwrap_err()
            .to_string();
        assert!(error.contains("controller of talos cluster 'k8s'"));
    }

    #[test]
    fn only_cluster_workers_wait_for_kubernetes_instead_of_their_api_port() {
        let clusters = [TalosCluster {
            name: "k8s".to_owned(),
            controllers: vec!["cp1".to_owned(), "cp2".to_owned(), "cp3".to_owned()],
            workers: vec!["w1".to_owned()],
            ..TalosCluster::default()
        }];
        assert!(readiness_waits_for_kubernetes(&clusters, "w1"));
        assert!(!readiness_waits_for_kubernetes(&clusters, "cp1"));
        assert!(!readiness_waits_for_kubernetes(&clusters, "standalone"));
    }

    fn plan_using(oidc: Option<ClusterMember>, vault: Option<ClusterMember>) -> ClusterPlan {
        ClusterPlan {
            name: "k8s".to_owned(),
            endpoint: Ipv4Addr::new(172, 31, 1, 21),
            vip: None,
            cni: ClusterCni::Flannel,
            load_balancer_addresses: None,
            control_planes: vec![ClusterMember {
                name: "cp1".to_owned(),
                ipv4: Ipv4Addr::new(172, 31, 1, 21),
            }],
            workers: vec![],
            oidc,
            vault,
        }
    }

    #[test]
    fn clusters_log_in_through_their_keycloaks_sherpa_realm() {
        let plan = plan_using(
            Some(ClusterMember {
                name: "kc01".to_owned(),
                ipv4: Ipv4Addr::new(172, 31, 1, 23),
            }),
            None,
        );
        assert_eq!(
            oidc_config(&plan, "lab-ca"),
            Some(ClusterOidcConfig {
                issuer_url: "https://172.31.1.23:8443/realms/sherpa".to_owned(),
                ca_cert: "lab-ca",
            })
        );
        assert_eq!(vault_config(&plan, "lab-ca"), None);
    }

    #[test]
    fn clusters_reach_their_vault_through_their_own_auth_mount() {
        let plan = plan_using(
            None,
            Some(ClusterMember {
                name: "vault01".to_owned(),
                ipv4: Ipv4Addr::new(172, 31, 1, 24),
            }),
        );
        assert_eq!(
            vault_config(&plan, "lab-ca"),
            Some(ClusterVaultConfig {
                address: "https://172.31.1.24:8200".to_owned(),
                ca_cert: "lab-ca",
                auth_mount: "kubernetes-k8s".to_owned(),
            })
        );
        assert_eq!(oidc_config(&plan, "lab-ca"), None);
    }
}
