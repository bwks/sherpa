//! Provisioners, run by the CLI for lab nodes.
//!
//! The ssh provisioner's data never goes to the Sherpa server: the CLI connects to nodes
//! itself, through the server as an SSH jump host. The container provisioner's files are sent
//! to the server outside the manifest and kept in RAM there.

mod client_files;
mod container;
mod env_file;
mod output;
mod remote;
mod run;
mod ssh;
mod transport;

pub use container::{container_results, container_secrets, node_secrets};
pub use run::provision_lab;
