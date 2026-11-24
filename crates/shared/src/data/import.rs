use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::node::{NodeConfig, NodeKind, NodeModel};

/// Request type for listing images with optional filters
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListImagesRequest {
    pub model: Option<NodeModel>,
    pub kind: Option<NodeKind>,
}

/// Summary of an imported image for display purposes
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImageSummary {
    pub model: NodeModel,
    pub kind: NodeKind,
    pub version: String,
    pub default: bool,
}

/// Response from listing images
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListImagesResponse {
    pub images: Vec<ImageSummary>,
    pub total: usize,
}

/// Request type for showing image details
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ShowImageRequest {
    pub model: NodeModel,
    #[serde(default)]
    pub kind: Option<NodeKind>,
    pub version: Option<String>,
}

/// Response from showing image details
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ShowImageResponse {
    pub image: NodeConfig,
}

/// Request type for importing a disk image. It denies unknown fields, so an import that sends
/// an image setting it does not take, such as a ZTP username, password or password auth, is
/// refused with serde's error naming the field instead of importing without it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportRequest {
    pub model: NodeModel,
    #[serde(default)]
    pub kind: Option<NodeKind>,
    pub version: String,
    pub src: String,
    #[serde(default)]
    pub default: bool,
}

/// Response from an image import operation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportResponse {
    pub success: bool,
    pub model: NodeModel,
    pub kind: NodeKind,
    pub version: String,
    pub image_path: String,
    pub db_tracked: bool,
    #[serde(default)]
    pub default: bool,
}

/// Request type for scanning for images that are present but not recorded: disk images in the
/// images directory, and container images in local Docker.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScanImagesRequest {
    pub kind: Option<NodeKind>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub model: Option<NodeModel>,
    #[serde(default)]
    pub repo: Option<String>,
}

/// A single scanned image result
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScannedImage {
    pub model: NodeModel,
    pub version: String,
    pub kind: NodeKind,
    pub status: String,
    #[serde(default)]
    pub failed: bool,
}

/// A model an image scan could not match images to, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkippedModel {
    pub model: NodeModel,
    pub reason: String,
}

/// Response from scanning for images
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScanImagesResponse {
    pub scanned: Vec<ScannedImage>,
    #[serde(default)]
    pub skipped: Vec<SkippedModel>,
    pub total_imported: usize,
}

/// Request type for deleting an imported image
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeleteImageRequest {
    pub model: NodeModel,
    #[serde(default)]
    pub kind: Option<NodeKind>,
    pub version: String,
}

/// Response from an image delete operation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeleteImageResponse {
    pub success: bool,
    pub model: NodeModel,
    pub kind: NodeKind,
    pub version: String,
    pub image_deleted: bool,
    pub db_deleted: bool,
    #[serde(default)]
    pub new_default: Option<String>,
}

/// Request type for setting the default version of an image
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetDefaultImageRequest {
    pub model: NodeModel,
    #[serde(default)]
    pub kind: Option<NodeKind>,
    pub version: String,
}

/// Response from setting the default image version
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetDefaultImageResponse {
    pub success: bool,
    pub model: NodeModel,
    pub kind: NodeKind,
    pub version: String,
}

/// Request type for pulling a container image from an OCI registry
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ContainerPullRequest {
    pub model: super::NodeModel,
    #[serde(default)]
    pub kind: Option<NodeKind>,
    #[serde(default)]
    pub repo: Option<String>,
    pub tag: String,
    #[serde(default)]
    pub default: bool,
}

/// Response from a container image pull operation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ContainerPullResponse {
    pub success: bool,
    pub model: String,
    pub repo: String,
    pub tag: String,
    pub db_tracked: bool,
    pub message: String,
}

/// Request type for downloading a VM image from a URL
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DownloadImageRequest {
    pub model: NodeModel,
    #[serde(default)]
    pub kind: Option<NodeKind>,
    pub version: String,
    pub url: Option<String>,
    #[serde(default)]
    pub default: bool,
}
