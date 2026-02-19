use serde::{Deserialize, Serialize};
use surrealdb_types::{Datetime, RecordId, SurrealValue};

use super::BridgeKind;

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
}

#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
pub struct DbNode {
    pub id: Option<RecordId>,
    pub name: String,
    pub config: RecordId,
    pub index: u16,
    pub lab: RecordId,
    pub mgmt_ipv4: Option<String>,
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
