use serde::{Deserialize, Serialize};

use surrealdb::RecordId;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DbUser {
    pub id: Option<RecordId>,
    pub ssh_keys: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DbLab {
    pub id: Option<RecordId>,
    pub lab_id: String,
    pub name: String,
    pub user: RecordId,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NodeVariant {
    pub id: Option<RecordId>,
    pub model: RecordId,
    pub kind: RecordId,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DbNode {
    pub id: Option<RecordId>,
    pub name: String,
    pub variant: RecordId, // record<node_variant>
    pub index: u16,
    pub lab: RecordId, // record<lab>
}
