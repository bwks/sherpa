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

impl fmt::Display for ClusterRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClusterRole::ControlPlane => write!(f, "control_plane"),
            ClusterRole::Worker => write!(f, "worker"),
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
}
