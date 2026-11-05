use std::fmt;

use schemars::JsonSchema;
use serde_derive::{Deserialize, Serialize};

/// Role of a node in a Kubernetes cluster declared in the manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClusterRole {
    /// Runs etcd and the Kubernetes control plane.
    ControlPlane,
    /// Runs workloads only.
    Worker,
}

/// The CNI of a Kubernetes cluster declared in the manifest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClusterCni {
    /// Flannel, deployed by Talos.
    #[default]
    Flannel,
    /// Cilium, replacing kube-proxy, with Gateway API, load balancer addresses announced on
    /// the management network, and Hubble.
    Cilium,
}

impl ClusterCni {
    pub fn is_flannel(&self) -> bool {
        *self == ClusterCni::Flannel
    }
}

impl fmt::Display for ClusterRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClusterRole::ControlPlane => write!(f, "control_plane"),
            ClusterRole::Worker => write!(f, "worker"),
        }
    }
}

impl fmt::Display for ClusterCni {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClusterCni::Flannel => write!(f, "flannel"),
            ClusterCni::Cilium => write!(f, "cilium"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_use_snake_case_names() {
        for (role, name) in [
            (ClusterRole::ControlPlane, "control_plane"),
            (ClusterRole::Worker, "worker"),
        ] {
            assert_eq!(role.to_string(), name);
            assert_eq!(serde_json::to_string(&role).unwrap(), format!("\"{name}\""));
        }
    }

    #[test]
    fn cnis_use_snake_case_names_and_default_to_flannel() {
        for (cni, name) in [
            (ClusterCni::Flannel, "flannel"),
            (ClusterCni::Cilium, "cilium"),
        ] {
            assert_eq!(cni.to_string(), name);
            assert_eq!(serde_json::to_string(&cni).unwrap(), format!("\"{name}\""));
        }
        assert_eq!(ClusterCni::default(), ClusterCni::Flannel);
        assert!(ClusterCni::Flannel.is_flannel());
        assert!(!ClusterCni::Cilium.is_flannel());
    }
}
