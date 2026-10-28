use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ClusterInfo, LabInfo};

/// Response containing lab files for download
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DownloadLabResponse {
    pub lab_info: LabInfo,
    pub ssh_config: String,
    /// The lab's `known_hosts` file: node host keys the server recorded. Empty when none
    /// were, or from an older server.
    #[serde(default)]
    pub known_hosts: String,
    pub ssh_private_key: String,
    /// Kubernetes clusters built in the lab, with their credentials.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clusters: Vec<ClusterInfo>,
}
