use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{LinkReport, SecretFile};

/// Request type for redeploying a single node
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RedeployRequest {
    pub lab_id: String,
    pub node_name: String,
    pub manifest: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets: Option<Vec<SecretFile>>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<LinkReport>,
}
