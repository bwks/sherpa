use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Request type for redeploying a single node
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RedeployRequest {
    pub lab_id: String,
    pub node_name: String,
    pub manifest: serde_json::Value,
    /// Authenticated requester, set by server entry points; client input is ignored.
    pub username: String,
}

/// Response type for redeploy operation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RedeployResponse {
    pub success: bool,
    pub node_name: String,
    pub message: String,
    pub total_time_secs: u64,
}
