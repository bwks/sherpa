use super::{LinkReport, TailnetStatus};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Result of a single node action (shutdown, start, etc.)
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct NodeActionResult {
    pub name: String,
    pub success: bool,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<LinkReport>,
}

/// Response for lab-wide node actions (down/resume)
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LabNodeActionResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailscale: Option<TailnetStatus>,
    pub results: Vec<NodeActionResult>,
}

/// Response for stopping the operations running on a lab.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CancelLabResponse {
    pub lab_id: String,
    pub stopped: bool,
}
