//! Client-side provisioners, run on lab nodes once they are ready.
//!
//! Provisioner data never goes to the Sherpa server: the CLI connects to nodes itself, through
//! the server as an SSH jump host.

mod remote;
mod run;
mod ssh;
mod transport;

pub use run::provision_lab;
