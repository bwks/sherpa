use super::{LinkReport, TailnetStatus};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Result of a single node action (shutdown, start, etc.)
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct NodeActionResult {
    pub name: String,
    pub success: bool,
    pub message: String,
    /// The node's P2p links, when the action made its end again (a cold boot or a container
    /// start). Empty otherwise. A link that is not attached does not fail the node.
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
    /// Whether an operation was running and has now stopped.
    pub stopped: bool,
}
