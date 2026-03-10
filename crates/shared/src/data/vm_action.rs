use serde::{Deserialize, Serialize};

/// Result of a single node action (shutdown, start, etc.)
#[derive(Debug, Serialize, Deserialize)]
pub struct NodeActionResult {
    pub name: String,
    pub success: bool,
    pub message: String,
}

/// Response for lab-wide node actions (down/resume)
#[derive(Debug, Serialize, Deserialize)]
pub struct LabNodeActionResponse {
    pub results: Vec<NodeActionResult>,
}
