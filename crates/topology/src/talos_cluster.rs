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

    #[test]
    fn manifest_clusters_011_a_nodes_cluster_and_role_are_those_of_the_list_it_is_in() {
        let k8s: TalosCluster = toml::from_str(
            r#"
            name = "k8s"
            controllers = ["cp2", "cp1", "cp3"]
            workers = ["w1"]
            "#,
        )
        .unwrap();
        let clusters = vec![
            k8s,
            TalosCluster {
                name: "dev".to_owned(),
                controllers: vec!["dev-cp1".to_owned()],
                ..TalosCluster::default()
            },
        ];

        assert_eq!(
            clusters[0].members(),
            vec![
                ("cp2", ClusterRole::ControlPlane),
                ("cp1", ClusterRole::ControlPlane),
                ("cp3", ClusterRole::ControlPlane),
                ("w1", ClusterRole::Worker),
            ]
        );
        for (node, cluster, role) in [
            ("cp1", "k8s", ClusterRole::ControlPlane),
            ("w1", "k8s", ClusterRole::Worker),
            ("dev-cp1", "dev", ClusterRole::ControlPlane),
        ] {
            let (found, found_role) = find_talos_cluster(&clusters, node).unwrap();
            assert_eq!((found.name.as_str(), found_role), (cluster, role), "{node}");
        }
        assert!(find_talos_cluster(&clusters, "standalone").is_none());
    }
}
