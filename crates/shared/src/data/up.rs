use serde::{Deserialize, Serialize};

/// Request type for starting a lab
/// Note: manifest is passed as JSON Value to avoid cyclic dependencies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpRequest {
    pub lab_id: String,
    pub manifest: serde_json::Value,
    /// Username of the requesting user
    /// TODO: This is username-without-authentication. When adding authentication layer,
    /// replace this with verified identity from auth token/session.
    pub username: String,
}

/// Response type for lab startup operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpResponse {
    pub success: bool,
    pub lab_id: String,
    pub lab_name: String,
    pub total_time_secs: u64,
    pub phases_completed: Vec<String>,
    pub summary: UpSummary,
    pub nodes: Vec<NodeInfo>,
    pub errors: Vec<UpError>,
    pub ssh_config: String,
    pub ssh_private_key: String,
}

/// Summary of created resources
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpSummary {
    pub containers_created: usize,
    pub vms_created: usize,
    pub unikernels_created: usize,
    pub networks_created: usize,
    pub bridges_created: usize,
    pub interfaces_created: usize,
}

/// Information about a node in the lab
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub name: String,
    pub kind: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_port: Option<u16>,
}

/// Error tracking during lab startup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpError {
    pub phase: String,
    pub message: String,
    pub is_critical: bool,
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
