use anyhow::{Result, anyhow};
use shared::data::{DbLab, DbNode, DbUser, NodeConfig, RecordId};

/// Get a user's id from a user record.
///
/// # Errors
///
/// Fails when the record has no id, as one not yet stored. The error names the user only: the
/// record holds its password hash.
pub fn get_user_id(user: &DbUser) -> Result<RecordId> {
    user.id
        .clone()
        .ok_or_else(|| anyhow!("User record '{}' has no ID", user.username))
}

/// Get a lab's id from a lab record.
///
/// # Errors
///
/// Fails when the record has no id, as one not yet stored.
pub fn get_lab_id(lab: &DbLab) -> Result<RecordId> {
    lab.id
        .clone()
        .ok_or_else(|| anyhow!("Lab record {} ({}) has no ID", lab.name, lab.lab_id))
}

/// Get an image's id from a node image record.
///
/// # Errors
///
/// Fails when the record has no id, as one not yet stored. The error names the image only: the
/// record can hold a ZTP password.
pub fn get_image_id(config: &NodeConfig) -> Result<RecordId> {
    config.id.clone().ok_or_else(|| {
        anyhow!(
            "Node image record {} {} has no ID",
            config.model,
            config.version
        )
    })
}

/// Get a node's id from a node record.
///
/// # Errors
///
/// Fails when the record has no id, as one not yet stored.
pub fn get_node_id(node: &DbNode) -> Result<RecordId> {
    node.id
        .clone()
        .ok_or_else(|| anyhow!("Node record '{}' has no ID", node.name))
}
