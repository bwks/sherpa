//! Vault nodes Sherpa initialises and unseals, and the access it sets up for clusters.
//!
//! Sherpa initialises each Vault node that runs its generated config, keeps the unseal key and
//! root token with the lab's files, unseals it, and hands the credentials to the user. For each
//! Kubernetes cluster that uses the Vault, it enables an auth method that accepts the cluster's
//! service account tokens.

pub mod api;
pub mod cluster;
pub mod lab;

pub use cluster::{ClusterAccess, cluster_auth_mount, configure_cluster_access};
pub use lab::{ensure_vault_unsealed, read_vault_credentials, sherpa_initialises_vault, vault_dir};
