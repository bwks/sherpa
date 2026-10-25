use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::SecretFile;

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
}

/// Response type for redeploy operation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RedeployResponse {
    pub success: bool,
    pub node_name: String,
    pub message: String,
    pub total_time_secs: u64,
}
