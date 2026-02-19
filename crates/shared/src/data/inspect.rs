use serde::{Deserialize, Serialize};

use crate::data::{LabInfo, NodeKind, NodeModel};

/// Request type for inspecting a lab
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectRequest {
    pub lab_id: String,
    /// Username of the requesting user
    /// TODO: This is username-without-authentication. When adding authentication layer,
    /// replace this with verified identity from auth token/session.
    pub username: String,
}

/// Response structure for inspect operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectResponse {
    pub lab_info: LabInfo,
    pub devices: Vec<DeviceInfo>,
    pub inactive_devices: Vec<String>,
}

/// Information about a single device/node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    pub model: NodeModel,
    pub kind: NodeKind,
    pub active: bool,
    pub mgmt_ipv4: String,
    pub disks: Vec<String>,
}
