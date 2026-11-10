use std::collections::BTreeMap;

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
    /// Secret files for container nodes, by node name. Sent only by the CLI, outside the
    /// manifest, and never stored.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub node_secrets: BTreeMap<String, Vec<SecretFile>>,
    /// The hostname of the machine `sherpa up` runs on, stored with the lab. Sent only by the CLI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_host: Option<String>,
    /// The folder `sherpa up` runs in, stored with the lab. Sent only by the CLI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_folder: Option<String>,
}

/// Response type for lab startup operation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailscale: Option<TailnetStatus>,
    /// Whether every part of the lab came up: false when any error is critical. The lab is
    /// created either way, and can be used or destroyed.
    pub success: bool,
    pub lab_info: LabInfo,
    pub total_time_secs: u64,
    pub phases_completed: Vec<String>,
    pub summary: UpSummary,
    pub nodes: Vec<NodeInfo>,
    pub errors: Vec<UpError>,
    pub ssh_config: String,
    /// The lab's `known_hosts` file: node host keys the server recorded. Empty when none
    /// were, or from an older server.
    #[serde(default)]
    pub known_hosts: String,
    pub ssh_private_key: String,
    /// Kubernetes clusters built in the lab, with their credentials.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clusters: Vec<ClusterInfo>,
    /// The lab CA certificate (PEM) that signs node certificates. Empty from an older server.
    #[serde(default)]
    pub lab_ca: String,
    /// Vault nodes Sherpa initialised, with their credentials.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vaults: Vec<VaultInfo>,
    /// Keycloak nodes Sherpa set up a realm on, with the realm's users.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keycloaks: Vec<KeycloakInfo>,
}

/// A Keycloak node and the realm Sherpa made on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KeycloakInfo {
    /// Node name.
    pub name: String,
    /// Base address, `https://<management address>:8443`.
    pub address: String,
    /// The realm Sherpa made.
    pub realm: String,
    /// The realm's users and their passwords.
    pub users: Vec<KeycloakUser>,
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
    /// Node name.
    pub name: String,
    /// API address, `https://<management address>:8200`.
    pub address: String,
    pub root_token: String,
    /// The single key share that unseals Vault.
    pub unseal_key: String,
}

/// A Kubernetes cluster built in the lab, and the credentials to manage it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClusterInfo {
    pub name: String,
    /// Kubernetes API endpoint.
    pub endpoint: String,
    /// `talosctl` configuration with an `os:admin` certificate.
    pub talosconfig: String,
    /// `kubectl` configuration with a cluster-admin certificate.
    pub kubeconfig: String,
    /// Address of a Cilium cluster's Gateway.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<String>,
    /// Address of a Cilium cluster's Hubble UI.
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
    /// Critical: part of the lab failed and nothing will set it up again, such as a container
    /// that exited or a lab service that did not come up. Not critical: it may still come right,
    /// such as a node slow to answer, or it is advice.
    pub is_critical: bool,
}

/// Classifies each status message for appropriate emoji rendering on the client
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StatusKind {
    /// Action started or underway (🔄)
    Progress,
    /// Action completed (✅)
    Done,
    /// Neutral/informational message (ℹ️)
    #[default]
    Info,
    /// Polling/waiting for a condition (⏳)
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
    fn a_node_kind_is_sent_as_every_other_api_sends_it() {
        let node = NodeInfo {
            name: "dev01".to_string(),
            kind: NodeKind::VirtualMachine,
            model: NodeModel::UbuntuLinux,
            status: NodeState::Running,
            ip_address: None,
            ssh_port: None,
        };
        let json = serde_json::to_value(&node).unwrap();
        assert_eq!(json["kind"], "virtual_machine");
    }

    #[test]
    fn test_status_kind_serde_round_trip() {
        let kinds = [
            StatusKind::Progress,
            StatusKind::Done,
            StatusKind::Info,
            StatusKind::Waiting,
        ];
        for kind in &kinds {
            let json = serde_json::to_string(kind).unwrap();
            let deserialized: StatusKind = serde_json::from_str(&json).unwrap();
            assert_eq!(*kind, deserialized);
        }
    }

    #[test]
    fn test_status_kind_default_is_info() {
        assert_eq!(StatusKind::default(), StatusKind::Info);
    }

    #[test]
    fn test_status_kind_rename_all() {
        assert_eq!(
            serde_json::to_string(&StatusKind::Progress).unwrap(),
            r#""progress""#
        );
        assert_eq!(
            serde_json::to_string(&StatusKind::Done).unwrap(),
            r#""done""#
        );
        assert_eq!(
            serde_json::to_string(&StatusKind::Info).unwrap(),
            r#""info""#
        );
        assert_eq!(
            serde_json::to_string(&StatusKind::Waiting).unwrap(),
            r#""waiting""#
        );
    }

    #[test]
    fn test_status_kind_deserialize_default() {
        // Simulate a struct that uses #[serde(default)] for StatusKind
        #[derive(Deserialize)]
        struct TestMsg {
            #[serde(default)]
            kind: StatusKind,
        }
        let msg: TestMsg = serde_json::from_str(r"{}").unwrap();
        assert_eq!(msg.kind, StatusKind::Info);
    }

    #[test]
    fn test_up_phase_as_str() {
        assert_eq!(UpPhase::Setup.as_str(), "Setup");
        assert_eq!(UpPhase::ManifestValidation.as_str(), "Manifest Validation");
        assert_eq!(UpPhase::DatabaseRecords.as_str(), "Database Records");
        assert_eq!(UpPhase::LabNetworkSetup.as_str(), "Lab Network Setup");
        assert_eq!(
            UpPhase::LinkCreation.as_str(),
            "Point-to-Point Link Creation"
        );
        assert_eq!(
            UpPhase::ContainerNetworks.as_str(),
            "Container Link Networks"
        );
        assert_eq!(UpPhase::SharedBridges.as_str(), "Shared Bridge Creation");
        assert_eq!(
            UpPhase::ZtpGeneration.as_str(),
            "ZTP Configuration Generation"
        );
        assert_eq!(UpPhase::BootContainers.as_str(), "Boot Container Creation");
        assert_eq!(UpPhase::DiskCloning.as_str(), "Disk Cloning");
        assert_eq!(UpPhase::VmCreation.as_str(), "VM Creation");
        assert_eq!(UpPhase::SshConfig.as_str(), "SSH Config Generation");
        assert_eq!(UpPhase::NodeReadiness.as_str(), "Node Readiness Check");
    }

    #[test]
    fn test_up_phase_number_sequential() {
        let phases = [
            UpPhase::Setup,
            UpPhase::ManifestValidation,
            UpPhase::DatabaseRecords,
            UpPhase::LabNetworkSetup,
            UpPhase::LinkCreation,
            UpPhase::ContainerNetworks,
            UpPhase::SharedBridges,
            UpPhase::ZtpGeneration,
            UpPhase::BootContainers,
            UpPhase::DiskCloning,
            UpPhase::VmCreation,
            UpPhase::SshConfig,
            UpPhase::NodeReadiness,
        ];
        for (number, phase) in (1u8..).zip(phases.iter()) {
            assert_eq!(phase.number(), number);
        }
    }

    #[test]
    fn test_up_phase_total_phases_matches_enum_count() {
        assert_eq!(UpPhase::total_phases(), 13);
    }

    #[test]
    fn test_up_phase_number_matches_total() {
        // Last phase number should equal total_phases
        assert_eq!(UpPhase::NodeReadiness.number(), UpPhase::total_phases());
    }
}
