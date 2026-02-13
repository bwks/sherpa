use serde::{Deserialize, Serialize};

use crate::data::{LabInfo, NodeKind, NodeModel};

/// Response structure for inspect operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectResponse {
    pub lab_name: String,
    pub lab_id: String,
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
    pub mgmt_ip: String,
    pub disks: Vec<String>,
}
