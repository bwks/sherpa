#![deny(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(not(test), forbid(unsafe_code))]

mod as_written;
mod bridge;
mod file_contents;
mod link;
mod manifest;
mod node;
mod provisioner;
mod resolve;
mod talos_cluster;
pub mod validate;

// re-export
pub use as_written::{ManifestAsWritten, NodeAsWritten};
pub use bridge::{Bridge, BridgeDetailed, BridgeExpanded, BridgeLink, BridgeLinkDetailed};
pub use file_contents::NodeFileContents;
pub use link::ManifestImpairment;
pub use link::{Link2, LinkDetailed, LinkExpanded};
pub use manifest::{Manifest, TailscaleConfig};
pub use node::{
    Node, NodeContainer, NodeExpanded, NodeReadyCheck, NodeUnikernel, NodeVm, NodeZtp,
    StartupScript, SystemdUnit, SystemdUnitData, TextFile, TextFileData, VolumeMount,
};
pub use provisioner::{
    ContainerProvisioner, ContainerSecretFile, Provisioner, ProvisionerFile, SshProvisioner,
};
pub use resolve::{process_manifest_bridges, process_manifest_links};
pub use talos_cluster::{TalosCluster, find_talos_cluster};
