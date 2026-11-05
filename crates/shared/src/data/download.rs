use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ClusterInfo, KeycloakInfo, LabInfo, VaultInfo};

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
    /// The lab CA certificate (PEM). Empty from an older server or a lab without one.
    #[serde(default)]
    pub lab_ca: String,
    /// Vault nodes Sherpa initialised, with their credentials.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vaults: Vec<VaultInfo>,
    /// Keycloak nodes Sherpa set up a realm on, with the realm's users.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keycloaks: Vec<KeycloakInfo>,
}
