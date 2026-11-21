use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use shared::data::StatusKind;
use shared::error::RpcErrorCode;

/// RPC error information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    /// JSON-RPC error code
    pub code: RpcErrorCode,
    /// Human-readable error message
    pub message: String,
    /// Additional debug context (<file:line>, stack trace)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// JSON-RPC 2.0 `data`: structured detail for codes that define it, such as the
    /// `MemoryShortfall` of `InsufficientMemory` and the `CpuPinConflict` of `CpuPinsHeld`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Message sent from server to client
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Status update
    Status {
        message: String,
        timestamp: Timestamp,
        kind: StatusKind,
        #[serde(skip_serializing_if = "Option::is_none")]
        phase: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        progress: Option<StatusProgress>,
    },

    /// Operation result
    Result {
        success: bool,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
    },

    /// RPC response
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
    /// RPC request
    RpcRequest {
        id: String,
        method: String,
        params: serde_json::Value,
    },
}
