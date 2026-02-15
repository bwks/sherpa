use serde::{Deserialize, Serialize};
use surrealdb::RecordId;

use super::BridgeKind;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DbUser {
    pub id: Option<RecordId>,
    pub username: String,
    pub password_hash: String,
    pub is_admin: bool,
    pub ssh_keys: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DbLab {
    pub id: Option<RecordId>,
    pub lab_id: String,
    pub name: String,
    pub user: RecordId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DbNode {
    pub id: Option<RecordId>,
    pub name: String,
    pub config: RecordId,
    pub index: u16,
    pub lab: RecordId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DbBridge {
    pub id: Option<RecordId>,
    pub index: u16,
    pub bridge_name: String,
    pub network_name: String,
    pub lab: RecordId,
    pub nodes: Vec<RecordId>,
}
