//! Kubernetes clusters built from Talos Linux nodes.
//!
//! Sherpa generates each cluster's secrets and node machine configs, bootstraps etcd on one
//! control plane, waits for Kubernetes, and writes a talosconfig and kubeconfig for the user.

pub mod bootstrap;
pub mod plan;
pub mod secrets;
