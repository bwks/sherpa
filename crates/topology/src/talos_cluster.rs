use serde_derive::{Deserialize, Serialize};

use shared::data::{ClusterCni, ClusterRole};

/// A Kubernetes cluster built from Talos Linux nodes in the lab.
///
/// The cluster names its member nodes: `controllers` run etcd and the Kubernetes control
/// plane, `workers` run workloads. Sherpa generates the cluster's secrets and machine configs,
/// bootstraps etcd on the first controller, and returns its talosconfig and kubeconfig.
#[derive(Clone, Debug, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TalosCluster {
    /// Cluster name, also used as the Kubernetes cluster name.
    pub name: String,
    /// Names of the nodes that run the control plane, in bootstrap order.
    pub controllers: Vec<String>,
    /// Names of the nodes that run workloads only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workers: Vec<String>,
    /// The cluster's CNI: Flannel unless set.
    #[serde(default, skip_serializing_if = "ClusterCni::is_flannel")]
    pub cni: ClusterCni,
    /// A `keycloak_server` node whose `sherpa` realm users log in to the cluster with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc: Option<String>,
    /// A `hashicorp_vault` node the cluster's workloads read secrets from, through the Vault
    /// Secrets Operator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault: Option<String>,
}

impl TalosCluster {
    /// The role of `node_name` in this cluster, if it is a member.
    #[must_use]
    pub fn role_of(&self, node_name: &str) -> Option<ClusterRole> {
        if self.controllers.iter().any(|name| name == node_name) {
            Some(ClusterRole::ControlPlane)
        } else if self.workers.iter().any(|name| name == node_name) {
            Some(ClusterRole::Worker)
        } else {
            None
        }
    }

    /// Every member with its role: controllers first, in bootstrap order, then workers.
    #[must_use]
    pub fn members(&self) -> Vec<(&str, ClusterRole)> {
        let mut members = Vec::with_capacity(self.controllers.len() + self.workers.len());
        for name in &self.controllers {
            members.push((name.as_str(), ClusterRole::ControlPlane));
        }
        for name in &self.workers {
            members.push((name.as_str(), ClusterRole::Worker));
        }
        members
    }
}

/// The cluster `node_name` belongs to and its role there, if it is in one.
#[must_use]
pub fn find_talos_cluster<'a>(
    clusters: &'a [TalosCluster],
    node_name: &str,
) -> Option<(&'a TalosCluster, ClusterRole)> {
    for cluster in clusters {
        if let Some(role) = cluster.role_of(node_name) {
            return Some((cluster, role));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cluster() -> TalosCluster {
        toml::from_str(
            r#"
            name = "k8s"
            controllers = ["cp1", "cp2", "cp3"]
            workers = ["w1"]
            "#,
        )
        .unwrap()
    }

    #[test]
    fn roles_come_from_the_member_lists() {
        let cluster = cluster();
        assert_eq!(cluster.role_of("cp2"), Some(ClusterRole::ControlPlane));
        assert_eq!(cluster.role_of("w1"), Some(ClusterRole::Worker));
        assert_eq!(cluster.role_of("other"), None);
    }

    #[test]
    fn members_list_controllers_in_order_then_workers() {
        assert_eq!(
            cluster().members(),
            vec![
                ("cp1", ClusterRole::ControlPlane),
                ("cp2", ClusterRole::ControlPlane),
                ("cp3", ClusterRole::ControlPlane),
                ("w1", ClusterRole::Worker),
            ]
        );
    }

    #[test]
    fn workers_are_optional_and_controllers_are_required() {
        let cluster: TalosCluster = toml::from_str(
            r#"
            name = "solo"
            controllers = ["cp1"]
            "#,
        )
        .unwrap();
        assert_eq!(cluster.workers, [] as [std::string::String; 0]);
        assert!(toml::from_str::<TalosCluster>(r#"name = "none""#).is_err());
    }

    #[test]
    fn the_cni_defaults_to_flannel() {
        assert_eq!(cluster().cni, ClusterCni::Flannel);
        let cilium: TalosCluster = toml::from_str(
            r#"
            name = "k8s"
            controllers = ["cp1"]
            cni = "cilium"
            "#,
        )
        .unwrap();
        assert_eq!(cilium.cni, ClusterCni::Cilium);
        assert!(
            toml::from_str::<TalosCluster>(
                r#"
                name = "k8s"
                controllers = ["cp1"]
                cni = "calico"
                "#
            )
            .is_err()
        );
    }

    #[test]
    fn oidc_and_vault_name_lab_nodes() {
        assert_eq!(cluster().oidc, None);
        assert_eq!(cluster().vault, None);
        let cluster: TalosCluster = toml::from_str(
            r#"
            name = "k8s"
            controllers = ["cp1"]
            oidc = "kc01"
            vault = "vault01"
            "#,
        )
        .unwrap();
        assert_eq!(cluster.oidc.as_deref(), Some("kc01"));
        assert_eq!(cluster.vault.as_deref(), Some("vault01"));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        assert!(
            toml::from_str::<TalosCluster>(
                r#"
                name = "k8s"
                controllers = ["cp1"]
                nodes = []
                "#
            )
            .is_err()
        );
    }

    #[test]
    fn find_talos_cluster_returns_the_cluster_and_role() {
        let clusters = vec![
            cluster(),
            TalosCluster {
                name: "dev".to_owned(),
                controllers: vec!["dev-cp1".to_owned()],
                workers: vec![],
                ..TalosCluster::default()
            },
        ];
        let (found, role) = find_talos_cluster(&clusters, "dev-cp1").unwrap();
        assert_eq!(found.name, "dev");
        assert_eq!(role, ClusterRole::ControlPlane);
        assert!(find_talos_cluster(&clusters, "standalone").is_none());
    }
}
