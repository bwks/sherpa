use serde::{Deserialize, Serialize};
use surrealdb_types::{Datetime, RecordId, SurrealValue};

use super::{BridgeKind, LabState, NodeState};

#[derive(Debug, Serialize, Deserialize, Clone, SurrealValue)]
pub struct DbUser {
    pub id: Option<RecordId>,
    pub username: String,
    pub password_hash: String,
    pub is_admin: bool,
    pub ssh_keys: Vec<String>,
    pub created_at: Datetime,
    pub updated_at: Datetime,
}

#[derive(Debug, Serialize, Deserialize, Clone, SurrealValue)]
pub struct DbLab {
    pub id: Option<RecordId>,
    pub lab_id: String,
    pub name: String,
    pub user: RecordId,
    pub loopback_network: String,
    pub management_network: String,
    pub gateway_ipv4: String,
    pub router_ipv4: String,
    pub management_network_v6: Option<String>,
    pub gateway_ipv6: Option<String>,
    pub router_ipv6: Option<String>,
    pub loopback_network_v6: Option<String>,
    #[serde(default)]
    pub status: LabState,
}

#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
pub struct DbNode {
    pub id: Option<RecordId>,
    pub name: String,
    pub image: RecordId,
    pub index: u16,
    pub lab: RecordId,
    pub mgmt_ipv4: Option<String>,
    pub mgmt_ipv6: Option<String>,
    pub mgmt_mac: Option<String>,
    pub state: NodeState,
}

#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
pub struct DbLink {
    pub id: Option<RecordId>,
    pub index: u16,
    pub kind: BridgeKind,
    pub node_a: RecordId,
    pub node_b: RecordId,
    pub int_a: String,
    pub int_b: String,
    pub lab: RecordId,
    pub bridge_a: String,
    pub bridge_b: String,
    pub veth_a: String,
    pub veth_b: String,
    /// Tap device name for node_a side (P2p links only).
    #[serde(default)]
    pub tap_a: String,
    /// Tap device name for node_b side (P2p links only).
    #[serde(default)]
    pub tap_b: String,
    /// Link impairment: one-way delay in microseconds.
    #[serde(default)]
    pub delay_us: u32,
    /// Link impairment: delay jitter in microseconds.
    #[serde(default)]
    pub jitter_us: u32,
    /// Link impairment: packet loss percent (0.0-100.0).
    #[serde(default)]
    pub loss_percent: f32,
    /// Link impairment: packet reorder percent (0.0-100.0).
    #[serde(default)]
    pub reorder_percent: f32,
    /// Link impairment: bit-flip corruption percent (0.0-100.0).
    #[serde(default)]
    pub corrupt_percent: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
pub struct DbBridge {
    pub id: Option<RecordId>,
    pub index: u16,
    pub bridge_name: String,
    pub network_name: String,
    pub lab: RecordId,
    pub nodes: Vec<RecordId>,
}
