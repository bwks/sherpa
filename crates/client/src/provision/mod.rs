//! Provisioners, run by the CLI for lab nodes.
//!
//! The ssh provisioner's data never goes to the Sherpa server: the CLI connects to nodes
//! itself, directly, or with `--ssh-jump` through the server as an SSH jump host, which only
//! carries the encrypted connection. The container provisioner's files are sent to the server
//! outside the manifest and kept in RAM there.

mod client_files;
mod container;
mod env_file;
mod output;
mod remote;
mod run;
mod ssh;
mod transport;

pub use client_files::resolve_client_path;
pub use container::{container_results, container_secrets, node_secrets};
pub use env_file::client_variable;
pub use run::provision_lab;
