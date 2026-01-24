use serde::{Deserialize, Serialize};
use surrealdb::RecordId;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DbUser {
    pub id: Option<RecordId>,
    pub username: String,
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
pub struct DbNode {
    pub id: Option<RecordId>,
    pub name: String,
    pub variant: RecordId,
    pub index: u16,
    pub lab: RecordId,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DbLink {
    pub id: Option<RecordId>,
    pub link_id: u16,
    pub node_a: RecordId,
    pub node_b: RecordId,
    pub int_a: String,
    pub int_b: String,
    pub lab: RecordId,
}
