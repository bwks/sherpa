#![deny(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(not(test), forbid(unsafe_code))]

mod bridge;
mod link;
mod manifest;
mod node;

// re-export
pub use bridge::{
    Bridge, BridgeDetailed, BridgeExpanded, BridgeLink, BridgeLinkDetailed, BridgeLinkExpanded,
};
pub use link::{Link, Link2, LinkDetailed, LinkExpanded};
pub use manifest::Manifest;
pub use node::{Node, NodeExpanded, StartupScript, TextFile, TextFileData, VolumeMount};
