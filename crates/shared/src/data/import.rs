use serde::{Deserialize, Serialize};
use tabled::Tabled;

use super::node::{NodeKind, NodeModel};

/// Request type for listing images with optional filters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListImagesRequest {
    /// Optional model filter
    pub model: Option<NodeModel>,
    /// Optional kind filter (VirtualMachine, Container, Unikernel)
    pub kind: Option<NodeKind>,
}

/// Summary of an imported image for display purposes
#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct ImageSummary {
    #[tabled(rename = "Model")]
    pub model: NodeModel,
    #[tabled(rename = "Kind")]
    pub kind: NodeKind,
    #[tabled(rename = "Version")]
    pub version: String,
    #[tabled(rename = "Default")]
    pub default: bool,
}

/// Response from listing images
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListImagesResponse {
    /// List of image summaries
    pub images: Vec<ImageSummary>,
    /// Total number of images returned
    pub total: usize,
}

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

/// Request type for scanning on-disk images
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanImagesRequest {
    /// Optional kind filter (currently only VirtualMachine is supported)
    pub kind: Option<NodeKind>,
    /// If true, only report what would be imported without making changes
    #[serde(default)]
    pub dry_run: bool,
}

/// A single scanned image result
#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct ScannedImage {
    #[tabled(rename = "Model")]
    pub model: NodeModel,
    #[tabled(rename = "Version")]
    pub version: String,
    #[tabled(rename = "Kind")]
    pub kind: NodeKind,
    #[tabled(rename = "Status")]
    pub status: String,
}

/// Response from scanning on-disk images
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanImagesResponse {
    /// List of scanned image results
    pub scanned: Vec<ScannedImage>,
    /// Total number of images imported (new or updated)
    pub total_imported: usize,
}
