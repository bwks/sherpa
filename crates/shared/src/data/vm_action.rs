use serde::{Deserialize, Serialize};

/// Result of a single VM action (suspend, resume, etc.)
#[derive(Debug, Serialize, Deserialize)]
pub struct VmActionResult {
    pub name: String,
    pub success: bool,
    pub message: String,
}

/// Response for lab-wide VM actions (down/resume)
#[derive(Debug, Serialize, Deserialize)]
pub struct LabVmActionResponse {
    pub results: Vec<VmActionResult>,
}
