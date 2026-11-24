use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ClusterInfo, KeycloakInfo, LabInfo, SurrealDbInfo, VaultInfo};

/// Request for the files of a lab, which the CLI writes to the lab's folder.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DownloadLabRequest {
    pub lab_id: String,
}

/// Response containing lab files for download
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DownloadLabResponse {
    pub lab_info: LabInfo,
    pub ssh_config: String,
    #[serde(default)]
    pub known_hosts: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clusters: Vec<ClusterInfo>,
    #[serde(default)]
    pub lab_ca: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vaults: Vec<VaultInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keycloaks: Vec<KeycloakInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surreal_dbs: Vec<SurrealDbInfo>,
}
