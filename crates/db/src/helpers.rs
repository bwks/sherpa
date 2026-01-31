use anyhow::{anyhow, Result};
use data::{DbLab, DbNode, DbUser, NodeConfig, RecordId};

/// Get a user's id from a user record.
pub(crate) fn get_user_id(user: &DbUser) -> Result<RecordId> {
    user.id
        .as_ref()
        .cloned()
        .ok_or_else(|| anyhow!("User record has no id field:\n '{:#?}'", user))
}

/// Get a lab's id from a lab record.
pub(crate) fn get_lab_id(lab: &DbLab) -> Result<RecordId> {
    lab.id
        .as_ref()
        .cloned()
        .ok_or_else(|| anyhow!("Lab has no id field:\n {:#?}", lab))
}

/// Get a config's id from a config record.
pub(crate) fn get_config_id(config: &NodeConfig) -> Result<RecordId> {
    config
        .id
        .as_ref()
        .cloned()
        .ok_or_else(|| anyhow!("config has no id field:\n {:#?}", config))
}

/// Get a node's id from a node record.
pub(crate) fn get_node_id(node: &DbNode) -> Result<RecordId> {
    node.id
        .as_ref()
        .cloned()
        .ok_or_else(|| anyhow!("Node has no id field:\n {:#?}", node))
}
