use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// SSH host key recorded for a lab node, in OpenSSH public key form (`ssh-ed25519 AAAA...`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NodeHostKey {
    pub node_name: String,
    pub key: String,
}

/// Request to read a lab node's SSH host key again, for a client whose recorded key does not
/// match the one the node offered.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NodeHostKeysRequest {
    pub lab_id: String,
    pub node_name: String,
}

/// Response to a [`NodeHostKeysRequest`]: the lab's `known_hosts` file with the node's
/// current key.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NodeHostKeysResponse {
    pub known_hosts: String,
    /// Whether the node offered a different key than the server had recorded.
    pub changed: bool,
}
