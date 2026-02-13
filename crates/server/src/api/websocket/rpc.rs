use anyhow::{Context, Result};
use serde_json;

use crate::api::websocket::messages::{RpcError, ServerMessage};
use crate::daemon::state::AppState;
use crate::services::inspect;

/// Handle incoming RPC request and route to appropriate service
///
/// This function:
/// 1. Routes the RPC method to the appropriate service handler
/// 2. Converts service results to RpcResponse format
/// 3. Captures errors and converts them to RpcError format
pub async fn handle_rpc_request(
    id: String,
    method: String,
    params: serde_json::Value,
    state: &AppState,
) -> ServerMessage {
    match method.as_str() {
        "inspect" => handle_inspect(id, params, state).await,
        _ => {
            // Unknown method
            ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: -32601,
                    message: format!("Method '{}' not found", method),
                    context: None,
                }),
            }
        }
    }
}

/// Handle "inspect" RPC call
///
/// Expected params: {"lab_id": "string"}
async fn handle_inspect(id: String, params: serde_json::Value, state: &AppState) -> ServerMessage {
    // Parse params
    let lab_id = match params.get("lab_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: -32602,
                    message: "Invalid params: 'lab_id' (string) is required".to_string(),
                    context: None,
                }),
            };
        }
    };

    // Call service
    match inspect::inspect_lab(lab_id, state).await {
        Ok(response) => {
            // Convert response to JSON
            match serde_json::to_value(&response) {
                Ok(result) => ServerMessage::RpcResponse {
                    id,
                    result: Some(result),
                    error: None,
                },
                Err(e) => ServerMessage::RpcResponse {
                    id,
                    result: None,
                    error: Some(RpcError {
                        code: -32603,
                        message: "Failed to serialize response".to_string(),
                        context: Some(format!("{:?}", e)),
                    }),
                },
            }
        }
        Err(e) => {
            // Convert service error to RpcError
            let error_chain = format!("{:?}", e);
            ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: -32000,
                    message: "Inspect operation failed".to_string(),
                    context: Some(error_chain),
                }),
            }
        }
    }
}
