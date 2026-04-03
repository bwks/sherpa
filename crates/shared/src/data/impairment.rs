use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Request type for updating link impairment on a running lab.
/// Delay and jitter values are in milliseconds.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateImpairmentRequest {
    pub lab_id: String,
    pub link_index: u16,
    pub username: String,
    #[serde(default)]
    pub delay: u32,
    #[serde(default)]
    pub jitter: u32,
    #[serde(default)]
    pub loss_percent: f32,
    #[serde(default)]
    pub reorder_percent: f32,
    #[serde(default)]
    pub corrupt_percent: f32,
}

/// Response type for updating link impairment
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateImpairmentResponse {
    pub success: bool,
    pub message: String,
}
