use anyhow::{Context, Result};
use axum::extract::ws::Message;
use serde_json;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::api::websocket::connection::Connection;
use crate::api::websocket::messages::{RpcError, ServerMessage};
use crate::daemon::state::AppState;
use crate::services::{destroy, inspect, progress, up};
use shared::data;

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
        "destroy" => handle_destroy(id, params, state).await,
        // Note: "up" is handled separately via handle_streaming_rpc_request
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

/// Handle streaming RPC request (sends multiple messages during execution)
/// This is called directly from the handler for methods that need streaming
pub async fn handle_streaming_rpc_request(
    id: String,
    method: String,
    params: serde_json::Value,
    state: &AppState,
    connection: &Arc<Connection>,
) {
    match method.as_str() {
        "up" => handle_up(id, params, state, connection).await,
        _ => {
            // Unknown streaming method
            let response = ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: -32601,
                    message: format!("Streaming method '{}' not found", method),
                    context: None,
                }),
            };
            if let Ok(json) = serde_json::to_string(&response) {
                let _ = connection.send(Message::Text(json.into())).await;
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

/// Handle "destroy" RPC call
///
/// Expected params: {"lab_id": "string"}
async fn handle_destroy(id: String, params: serde_json::Value, state: &AppState) -> ServerMessage {
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
    match destroy::destroy_lab(lab_id, state).await {
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
                    message: "Destroy operation failed".to_string(),
                    context: Some(error_chain),
                }),
            }
        }
    }
}

/// Handle "up" RPC call (streaming - sends progress updates)
///
/// Expected params: {"lab_id": "string", "manifest": object}
async fn handle_up(
    id: String,
    params: serde_json::Value,
    state: &AppState,
    connection: &Arc<Connection>,
) {
    // Parse params
    let lab_id = match params.get("lab_id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            let response = ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: -32602,
                    message: "Invalid params: 'lab_id' (string) is required".to_string(),
                    context: None,
                }),
            };
            if let Ok(json) = serde_json::to_string(&response) {
                let _ = connection.send(Message::Text(json.into())).await;
            }
            return;
        }
    };

    let manifest_value = match params.get("manifest") {
        Some(v) => v.clone(),
        None => {
            let response = ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: -32602,
                    message: "Invalid params: 'manifest' (object) is required".to_string(),
                    context: None,
                }),
            };
            if let Ok(json) = serde_json::to_string(&response) {
                let _ = connection.send(Message::Text(json.into())).await;
            }
            return;
        }
    };

    // Create progress channel
    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel();

    // Spawn task to forward progress messages to WebSocket
    let conn_clone = Arc::clone(connection);
    let forward_task = tokio::spawn(async move {
        while let Some(msg) = progress_rx.recv().await {
            let _ = conn_clone.send(msg).await;
        }
    });

    // Create progress sender
    let progress = progress::ProgressSender::new(progress_tx);

    // Create UpRequest
    let request = data::UpRequest {
        lab_id,
        manifest: manifest_value,
    };

    // Call the up service
    let result = up::up_lab(request, state, progress).await;

    // Close the progress channel (forward_task will finish when channel closes)
    // The channel is automatically closed when progress_tx is dropped here

    // Wait for forward task to complete
    let _ = forward_task.await;

    // Send final RPC response
    let response = match result {
        Ok(up_response) => {
            // Convert response to JSON
            match serde_json::to_value(&up_response) {
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
                    message: "Up operation failed".to_string(),
                    context: Some(error_chain),
                }),
            }
        }
    };

    // Send final response
    if let Ok(json) = serde_json::to_string(&response) {
        let _ = connection.send(Message::Text(json.into())).await;
    }
}
