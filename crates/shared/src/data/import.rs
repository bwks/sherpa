use serde::{Deserialize, Serialize};
use tabled::Tabled;

use super::node::{NodeConfig, NodeKind, NodeModel};

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

/// Request type for showing image details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShowImageRequest {
    /// The node model to show details for
    pub model: NodeModel,
    /// Optional version filter (if omitted, shows the default version)
    pub version: Option<String>,
}

/// Response from showing image details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShowImageResponse {
    /// Full image configuration
    pub image: NodeConfig,
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
    /// Whether to set this image as the default version
    #[serde(default)]
    pub default: bool,
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

/// Request type for deleting an imported image
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteImageRequest {
    /// The node model of the image to delete
    pub model: NodeModel,
    /// Version string for the image to delete
    pub version: String,
}

/// Response from an image delete operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteImageResponse {
    /// Whether the delete succeeded
    pub success: bool,
    /// The node model that was deleted
    pub model: NodeModel,
    /// The node kind (VirtualMachine, Container, Unikernel)
    pub kind: NodeKind,
    /// The version that was deleted
    pub version: String,
    /// Whether the disk files were deleted
    pub disk_deleted: bool,
    /// Whether the database record was deleted
    pub db_deleted: bool,
}

/// Request type for setting the default version of an image
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetDefaultImageRequest {
    /// The node model to set the default for
    pub model: NodeModel,
    /// Version string to make the default
    pub version: String,
}

/// Response from setting the default image version
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetDefaultImageResponse {
    /// Whether the operation succeeded
    pub success: bool,
    /// The node model that was updated
    pub model: NodeModel,
    /// The node kind (VirtualMachine, Container, Unikernel)
    pub kind: NodeKind,
    /// The version that is now the default
    pub version: String,
}

/// Request type for pulling a container image from an OCI registry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerPullRequest {
    /// Node model this image belongs to
    pub model: super::NodeModel,
    /// Container image repository (e.g., ghcr.io/nokia/srlinux)
    pub repo: String,
    /// Container image tag (e.g., 1.2.3)
    pub tag: String,
    /// Set this image as the default version
    #[serde(default)]
    pub default: bool,
}

/// Response from a container image pull operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerPullResponse {
    /// Whether the pull succeeded
    pub success: bool,
    /// Node model
    pub model: String,
    /// Container image repository
    pub repo: String,
    /// Container image tag
    pub tag: String,
    /// Whether the image was tracked in the database
    pub db_tracked: bool,
    /// Human-readable status message
    pub message: String,
}

/// Request type for downloading a VM image from a URL
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadImageRequest {
    /// The node model to download the image for
    pub model: NodeModel,
    /// Version string for the image
    pub version: String,
    /// Optional download URL (auto-resolved for some models)
    pub url: Option<String>,
    /// Whether to set this image as the default version
    #[serde(default)]
    pub default: bool,
}
