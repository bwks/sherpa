use serde_derive::{Deserialize, Serialize};

/// A Kubernetes cluster built from Talos Linux nodes in the lab.
///
/// Nodes join a cluster by naming it in their `cluster` field. Sherpa generates the cluster's
/// secrets and machine configs, bootstraps it, and returns its talosconfig and kubeconfig.
#[derive(Clone, Debug, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Cluster {
    /// Cluster name, also used as the Kubernetes cluster name.
    pub name: String,
}
