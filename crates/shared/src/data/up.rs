use std::collections::BTreeMap;
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::lab::LabInfo;
use super::node::{NodeKind, NodeModel, NodeState};
use super::{SecretFile, TailnetAuthKey, TailnetStatus};

/// Request type for starting a lab
/// Note: manifest is passed as JSON Value to avoid cyclic dependencies
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailscale_auth_key: Option<TailnetAuthKey>,
    pub lab_id: String,
    pub manifest: serde_json::Value,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub node_secrets: BTreeMap<String, Vec<SecretFile>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_folder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_written: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_contents: Option<serde_json::Value>,
}

/// Response type for lab startup operation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailscale: Option<TailnetStatus>,
    pub success: bool,
    pub lab_info: LabInfo,
    pub total_time_secs: u64,
    pub phases_completed: Vec<String>,
    pub summary: UpSummary,
    pub nodes: Vec<NodeInfo>,
    pub errors: Vec<UpError>,
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

/// A login Sherpa generated for an application node's own administrator: a Keycloak node's
/// `master` realm admin or a `SurrealDB` node's root user. Each node has its own, kept with the
/// lab's files on the server and handed to the lab owner only. Its `Debug` form hides the
/// password.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AdminCredential {
    pub username: String,
    pub password: String,
}

impl fmt::Debug for AdminCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdminCredential")
            .field("username", &self.username)
            .field("password", &"<hidden>")
            .finish()
    }
}

/// A Keycloak node and the realm Sherpa made on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KeycloakInfo {
    pub name: String,
    pub address: String,
    pub realm: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin: Option<AdminCredential>,
    pub users: Vec<KeycloakUser>,
}

/// A `SurrealDB` node and its root user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SurrealDbInfo {
    pub name: String,
    pub address: String,
    pub admin: AdminCredential,
}

/// A user in a Keycloak realm Sherpa made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KeycloakUser {
    pub username: String,
    pub password: String,
}

/// A Vault node Sherpa initialised and unsealed, and the credentials to use it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VaultInfo {
    pub name: String,
    pub address: String,
    pub root_token: String,
    pub unseal_key: String,
}

/// A Kubernetes cluster built in the lab, and the credentials to manage it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClusterInfo {
    pub name: String,
    pub endpoint: String,
    pub talosconfig: String,
    pub kubeconfig: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hubble_ui: Option<String>,
}

/// Summary of created resources
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpSummary {
    pub containers_created: usize,
    pub vms_created: usize,
    pub unikernels_created: usize,
    pub networks_created: usize,
    pub bridges_created: usize,
    pub interfaces_created: usize,
}

/// Information about a node in the lab
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NodeInfo {
    pub name: String,
    pub kind: NodeKind,
    pub model: NodeModel,
    pub status: NodeState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_port: Option<u16>,
}

/// Error tracking during lab startup
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpError {
    pub phase: String,
    pub message: String,
    pub is_critical: bool,
}

/// Classifies each status message for appropriate emoji rendering on the client
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StatusKind {
    Progress,
    Done,
    #[default]
    Info,
    Waiting,
}

/// Client-side helper for deserializing streaming status messages from the server
#[derive(Debug, Clone, Deserialize)]
pub struct StatusMessage {
    pub r#type: String,
    pub message: String,
    #[serde(default)]
    pub kind: StatusKind,
    pub phase: Option<String>,
}

/// Phase enum for tracking progress
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpPhase {
    Setup,
    ManifestValidation,
    DatabaseRecords,
    LabNetworkSetup,
    LinkCreation,
    ContainerNetworks,
    SharedBridges,
    ZtpGeneration,
    BootContainers,
    DiskCloning,
    VmCreation,
    SshConfig,
    NodeReadiness,
}

impl UpPhase {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            UpPhase::Setup => "Setup",
            UpPhase::ManifestValidation => "Manifest Validation",
            UpPhase::DatabaseRecords => "Database Records",
            UpPhase::LabNetworkSetup => "Lab Network Setup",
            UpPhase::LinkCreation => "Point-to-Point Link Creation",
            UpPhase::ContainerNetworks => "Container Link Networks",
            UpPhase::SharedBridges => "Shared Bridge Creation",
            UpPhase::ZtpGeneration => "ZTP Configuration Generation",
            UpPhase::BootContainers => "Boot Container Creation",
            UpPhase::DiskCloning => "Disk Cloning",
            UpPhase::VmCreation => "VM Creation",
            UpPhase::SshConfig => "SSH Config Generation",
            UpPhase::NodeReadiness => "Node Readiness Check",
        }
    }

    #[must_use]
    pub fn number(&self) -> u8 {
        match self {
            UpPhase::Setup => 1,
            UpPhase::ManifestValidation => 2,
            UpPhase::DatabaseRecords => 3,
            UpPhase::LabNetworkSetup => 4,
            UpPhase::LinkCreation => 5,
            UpPhase::ContainerNetworks => 6,
            UpPhase::SharedBridges => 7,
            UpPhase::ZtpGeneration => 8,
            UpPhase::BootContainers => 9,
            UpPhase::DiskCloning => 10,
            UpPhase::VmCreation => 11,
            UpPhase::SshConfig => 12,
            UpPhase::NodeReadiness => 13,
        }
    }

    #[must_use]
    pub fn total_phases() -> u8 {
        13
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_jobs_003_a_status_message_with_no_kind_is_shown_as_information() {
        let message: StatusMessage =
            serde_json::from_str(r#"{"type":"status","message":"Booting"}"#).unwrap();
        assert_eq!(message.kind, StatusKind::Info);
    }
}
