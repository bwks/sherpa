use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{LinkReport, SecretFile};

/// Request type for redeploying a single node
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RedeployRequest {
    pub lab_id: String,
    pub node_name: String,
    pub manifest: serde_json::Value,
    /// Secret files for a container node, never stored. `None` keeps the node's current secret
    /// files, for redeploys from the web UI; the CLI always sends them, and an empty list
    /// removes them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets: Option<Vec<SecretFile>>,
    /// The parts of the manifest as the user wrote them, as for `up`. `None` keeps the node's
    /// stored ones, for redeploys from the web UI and older clients.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_written: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_contents: Option<serde_json::Value>,
}

/// Response type for redeploy operation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RedeployResponse {
    pub success: bool,
    pub node_name: String,
    pub message: String,
    pub total_time_secs: u64,
    /// The rebuilt node's P2p links. A link that is not attached does not fail the redeploy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<LinkReport>,
}
