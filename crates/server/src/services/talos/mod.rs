//! Kubernetes clusters built from Talos Linux nodes.
//!
//! Sherpa generates each cluster's secrets and node machine configs, bootstraps etcd on one
//! control plane, waits for Kubernetes, and writes a talosconfig and kubeconfig for the user.

pub mod bootstrap;
pub mod lab;
pub mod plan;
pub mod secrets;

pub use lab::{
    LabCluster, bring_up_clusters, ensure_redeployable, find_cluster, load_cluster,
    node_machine_config, prepare_clusters, read_cluster_credentials,
};
