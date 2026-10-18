#![deny(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(not(test), forbid(unsafe_code))]

mod bridge;
mod link;
mod manifest;
mod node;
mod talos_cluster;
pub mod validate;

// re-export
pub use bridge::{
    Bridge, BridgeDetailed, BridgeExpanded, BridgeLink, BridgeLinkDetailed, BridgeLinkExpanded,
};
pub use link::{Link, Link2, LinkDetailed, LinkExpanded};
pub use manifest::Manifest;
pub use node::{Node, NodeExpanded, StartupScript, TextFile, TextFileData, VolumeMount};
pub use talos_cluster::{TalosCluster, find_talos_cluster};
