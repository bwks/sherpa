use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::lab::LabInfo;
use super::node::{NodeModel, NodeState};

/// Request type for starting a lab
/// Note: manifest is passed as JSON Value to avoid cyclic dependencies
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpRequest {
    pub lab_id: String,
    pub manifest: serde_json::Value,
    /// Username of the requesting user
    /// TODO: This is username-without-authentication. When adding authentication layer,
    /// replace this with verified identity from auth token/session.
    pub username: String,
}

/// Response type for lab startup operation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpResponse {
    pub success: bool,
    pub lab_info: LabInfo,
    pub total_time_secs: u64,
    pub phases_completed: Vec<String>,
    pub summary: UpSummary,
    pub nodes: Vec<NodeInfo>,
    pub errors: Vec<UpError>,
    pub ssh_config: String,
    pub ssh_private_key: String,
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
    pub kind: String,
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

    pub fn total_phases() -> u8 {
        13
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "\"progress\""
        );
        assert_eq!(
            serde_json::to_string(&StatusKind::Done).unwrap(),
            "\"done\""
        );
        assert_eq!(
            serde_json::to_string(&StatusKind::Info).unwrap(),
            "\"info\""
        );
        assert_eq!(
            serde_json::to_string(&StatusKind::Waiting).unwrap(),
            "\"waiting\""
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
        let msg: TestMsg = serde_json::from_str(r#"{}"#).unwrap();
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
        assert_eq!(UpPhase::ContainerNetworks.as_str(), "Container Link Networks");
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
        for (i, phase) in phases.iter().enumerate() {
            assert_eq!(phase.number(), (i + 1) as u8);
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
