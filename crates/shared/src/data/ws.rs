use serde::{Deserialize, Serialize};

/// WebSocket connection confirmation message sent from server to client.
///
/// Serializes with `"type": "connected"` tag for wire compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename = "connected")]
pub struct ConnectedMsg {
    pub connection_id: String,
    pub server_version: String,
}
