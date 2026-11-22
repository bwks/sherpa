//! Vault nodes Sherpa initialises and unseals, and the access it sets up for clusters.
//!
//! Sherpa initialises each Vault node that runs its generated config, keeps the unseal key and
//! root token with the lab's files, unseals it, and hands the credentials to the user. It unseals
//! the node again after resume starts it, and whenever the scanner finds it sealed. For each
//! Kubernetes cluster that uses the Vault, it enables an auth method that accepts the cluster's
//! service account tokens.

pub mod api;
pub mod cluster;
pub mod lab;

pub use cluster::{ClusterAccess, cluster_auth_mount, configure_cluster_access};
pub use lab::{
    SealOutcome, ensure_vault_unsealed, manages_vault, read_vault_credentials, record_seal_outcome,
    sherpa_initialises_vault, unseal_started_vault, unseal_with_saved_key, vault_dir,
};
