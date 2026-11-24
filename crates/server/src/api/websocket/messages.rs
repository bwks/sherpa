use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use shared::data::StatusKind;
use shared::error::RpcErrorCode;

/// RPC error information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: RpcErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Message sent from server to client
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Status {
        message: String,
        timestamp: Timestamp,
        kind: StatusKind,
        #[serde(skip_serializing_if = "Option::is_none")]
        phase: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        progress: Option<StatusProgress>,
    },

    Result {
        success: bool,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
    },

    RpcResponse {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<RpcError>,
    },
}

/// Progress information for status updates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusProgress {
    pub current_phase: String,
    pub phase_number: u8,
    pub total_phases: u8,
}

/// Message sent from client to server
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    RpcRequest {
        id: String,
        method: String,
        params: serde_json::Value,
    },
}
