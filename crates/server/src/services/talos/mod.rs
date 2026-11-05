//! Kubernetes clusters built from Talos Linux nodes.
//!
//! Sherpa generates each cluster's secrets and node machine configs, bootstraps etcd on one
//! control plane, waits for Kubernetes, and writes a talosconfig and kubeconfig for the user.
//! A cluster can log in through a Keycloak node's realm and read secrets from a Vault node.

pub mod bootstrap;
pub mod lab;
pub mod plan;
pub mod secrets;

pub use lab::{
    LabCluster, bring_up_clusters, ensure_redeployable, find_cluster, find_member_cluster,
    grant_vault_access, load_cluster, load_clusters, node_machine_config, prepare_clusters,
    read_cluster_credentials, readiness_waits_for_kubernetes, request_shutdown,
};
