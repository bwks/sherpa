use serde::{Deserialize, Serialize};

use super::node::{NodeKind, NodeModel};

/// Request type for importing a disk image
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRequest {
    /// The node model to import the image for
    pub model: NodeModel,
    /// Version string for the image
    pub version: String,
    /// Source file path on the server filesystem
    pub src: String,
    /// Whether to set this version as the latest (creates symlink)
    pub latest: bool,
}

/// Response from an image import operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResponse {
    /// Whether the import succeeded
    pub success: bool,
    /// The node model that was imported
    pub model: NodeModel,
    /// The node kind (VirtualMachine, Container, Unikernel)
    pub kind: NodeKind,
    /// The version that was imported
    pub version: String,
    /// The destination path of the imported image
    pub image_path: String,
    /// Whether the image was tracked in the database
    pub db_tracked: bool,
}
