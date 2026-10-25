use super::TailnetStatus;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::data::{LabInfo, LabState, NodeKind, NodeModel, NodeState};

/// Request type for inspecting a lab
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InspectRequest {
    pub lab_id: String,
}

/// Response structure for inspect operation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InspectResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailscale: Option<TailnetStatus>,
    pub lab_info: LabInfo,
    pub lab_state: LabState,
    pub devices: Vec<DeviceInfo>,
    pub inactive_devices: Vec<String>,
    pub links: Vec<LinkInfo>,
    pub bridges: Vec<BridgeInfo>,
}

/// Information about a single device/node
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeviceInfo {
    pub name: String,
    pub model: NodeModel,
    pub kind: NodeKind,
    pub state: NodeState,
    pub mgmt_ipv4: String,
    pub mgmt_ipv6: Option<String>,
    /// The VM's VNC console port on the server.
    pub vnc_port: Option<u16>,
    pub disks: Vec<String>,
    /// vCPUs the node was given. `None` for containers, which Sherpa does not limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_count: Option<u8>,
    /// Memory in MB the node was given; `None` as for `cpu_count`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<u16>,
}

/// Display-ready information about a point-to-point link between two nodes
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LinkInfo {
    pub node_a_name: String,
    pub int_a: String,
    pub node_b_name: String,
    pub int_b: String,
    pub kind: String,
}

/// Display-ready information about a shared bridge connecting multiple nodes
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BridgeInfo {
    pub bridge_name: String,
    pub network_name: String,
    pub connected_nodes: Vec<String>,
}
