use serde::{Deserialize, Serialize};

use crate::data::{LabInfo, NodeKind, NodeModel, NodeState};

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
    pub links: Vec<LinkInfo>,
    pub bridges: Vec<BridgeInfo>,
}

/// Information about a single device/node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    pub model: NodeModel,
    pub kind: NodeKind,
    pub state: NodeState,
    pub mgmt_ipv4: String,
    pub disks: Vec<String>,
}

/// Display-ready information about a point-to-point link between two nodes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkInfo {
    pub node_a_name: String,
    pub int_a: String,
    pub node_b_name: String,
    pub int_b: String,
    pub kind: String,
}

/// Display-ready information about a shared bridge connecting multiple nodes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeInfo {
    pub bridge_name: String,
    pub network_name: String,
    pub connected_nodes: Vec<String>,
}
