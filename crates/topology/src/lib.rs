#![deny(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(not(test), forbid(unsafe_code))]

mod bridge;
mod link;
mod manifest;
mod node;
mod provisioner;
mod resolve;
mod talos_cluster;
pub mod validate;

// re-export
pub use bridge::{Bridge, BridgeDetailed, BridgeExpanded, BridgeLink, BridgeLinkDetailed};
pub use link::{Link2, LinkDetailed, LinkExpanded};
pub use manifest::Manifest;
pub use node::{
    AuthorizedKeyFile, BinaryFile, BinaryFileData, ExpandedSsh, Node, NodeContainer, NodeExpanded,
    NodeReadyCheck, NodeSsh, NodeUnikernel, NodeVm, NodeZtp, StartupScript, SystemdUnit,
    SystemdUnitData, TextFile, TextFileData, VolumeMount,
};
pub use provisioner::{
    ContainerProvisioner, ContainerSecretFile, Provisioner, ProvisionerFile, SshProvisioner,
};
pub use resolve::{process_manifest_bridges, process_manifest_links};
pub use talos_cluster::{TalosCluster, find_talos_cluster};
