use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// RPC request sent from client to server
#[derive(Debug, Clone, Serialize)]
pub struct RpcRequest {
    pub r#type: String, // Always "rpc_request"
    pub id: String,
    pub method: String,
    pub params: serde_json::Value,
}

/// RPC response received from server
#[derive(Debug, Clone, Deserialize)]
pub struct RpcResponse {
    pub id: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<RpcError>,
}

/// RPC error information
#[derive(Debug, Clone, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    pub context: Option<String>,
}

impl RpcRequest {
    /// Create a new RPC request with a generated UUID
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            r#type: "rpc_request".to_string(),
            id: Uuid::new_v4().to_string(),
            method: method.into(),
            params,
        }
    }
}
