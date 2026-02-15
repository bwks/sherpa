use axum::extract::ws::Message;
use serde_json;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::api::websocket::connection::Connection;
use crate::api::websocket::messages::{RpcError, ServerMessage};
use crate::auth::middleware;
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
        "auth.login" => handle_auth_login(id, params, state).await,
        "auth.validate" => handle_auth_validate(id, params, state).await,
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
/// Expected params: {"lab_id": "string", "token": "string"}
async fn handle_inspect(id: String, params: serde_json::Value, state: &AppState) -> ServerMessage {
    // Authenticate the request
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for inspect: {}", e);
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: -32401,
                    message: "Authentication required".to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Parse params
    let lab_id = match params.get("lab_id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
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

    // Check authorization: user must own the lab or be an admin
    match db::get_lab_owner_username(&state.db, &lab_id).await {
        Ok(owner_username) => {
            if !auth_ctx.can_access(&owner_username) {
                tracing::warn!(
                    "User '{}' attempted to inspect lab '{}' owned by '{}'",
                    auth_ctx.username,
                    lab_id,
                    owner_username
                );
                return ServerMessage::RpcResponse {
                    id,
                    result: None,
                    error: Some(RpcError {
                        code: -32403,
                        message: "Access denied: you do not have permission to access this lab"
                            .to_string(),
                        context: None,
                    }),
                };
            }
        }
        Err(e) => {
            // Lab not found
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: -32404,
                    message: format!("Lab not found: {}", lab_id),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    }

    // User is authenticated and authorized - use their username from the token
    let request = data::InspectRequest {
        lab_id,
        username: auth_ctx.username.clone(),
    };

    // Call service
    match inspect::inspect_lab(request, state).await {
        Ok(response) => {
            // Convert response to JSON
            match serde_json::to_value(&response) {
                Ok(result) => {
                    tracing::info!("User '{}' inspected lab successfully", auth_ctx.username);
                    ServerMessage::RpcResponse {
                        id,
                        result: Some(result),
                        error: None,
                    }
                }
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
/// Expected params: {"lab_id": "string", "token": "string"}
async fn handle_destroy(id: String, params: serde_json::Value, state: &AppState) -> ServerMessage {
    // Authenticate the request
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for destroy: {}", e);
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: -32401,
                    message: "Authentication required".to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Parse params
    let lab_id = match params.get("lab_id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
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

    // Check authorization: user must own the lab or be an admin
    match db::get_lab_owner_username(&state.db, &lab_id).await {
        Ok(owner_username) => {
            if !auth_ctx.can_access(&owner_username) {
                tracing::warn!(
                    "User '{}' attempted to destroy lab '{}' owned by '{}'",
                    auth_ctx.username,
                    lab_id,
                    owner_username
                );
                return ServerMessage::RpcResponse {
                    id,
                    result: None,
                    error: Some(RpcError {
                        code: -32403,
                        message: "Access denied: you do not have permission to access this lab"
                            .to_string(),
                        context: None,
                    }),
                };
            }
        }
        Err(e) => {
            // Lab not found
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: -32404,
                    message: format!("Lab not found: {}", lab_id),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    }

    // User is authenticated and authorized - use their username from the token
    let request = data::DestroyRequest {
        lab_id,
        username: auth_ctx.username.clone(),
    };

    // Call service
    match destroy::destroy_lab(request, state).await {
        Ok(response) => {
            // Convert response to JSON
            match serde_json::to_value(&response) {
                Ok(result) => {
                    tracing::info!("User '{}' destroyed lab successfully", auth_ctx.username);
                    ServerMessage::RpcResponse {
                        id,
                        result: Some(result),
                        error: None,
                    }
                }
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

/// Handle "up" RPC call (streaming)
///
/// Expected params: {"lab_id": "string", "manifest": object, "token": "string"}
async fn handle_up(
    id: String,
    params: serde_json::Value,
    state: &AppState,
    connection: &Arc<Connection>,
) {
    // Helper function to send error and return
    let send_error = |id: String, code: i32, message: String, context: Option<String>| async move {
        let response = ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(RpcError {
                code,
                message,
                context,
            }),
        };
        if let Ok(json) = serde_json::to_string(&response) {
            let _ = connection.send(Message::Text(json.into())).await;
        }
    };

    // Authenticate the request
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for up: {}", e);
            send_error(
                id,
                -32401,
                "Authentication required".to_string(),
                Some(format!("{:?}", e)),
            )
            .await;
            return;
        }
    };

    // Parse params
    let lab_id = match params.get("lab_id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            send_error(
                id,
                -32602,
                "Invalid params: 'lab_id' (string) is required".to_string(),
                None,
            )
            .await;
            return;
        }
    };

    let manifest_value = match params.get("manifest") {
        Some(v) => v.clone(),
        None => {
            send_error(
                id,
                -32602,
                "Invalid params: 'manifest' (object) is required".to_string(),
                None,
            )
            .await;
            return;
        }
    };

    // Check if lab already exists and verify ownership
    if let Ok(owner_username) = db::get_lab_owner_username(&state.db, &lab_id).await {
        // Lab exists - check authorization
        if !auth_ctx.can_access(&owner_username) {
            tracing::warn!(
                "User '{}' attempted to up lab '{}' owned by '{}'",
                auth_ctx.username,
                lab_id,
                owner_username
            );
            send_error(
                id,
                -32403,
                "Access denied: you do not have permission to access this lab".to_string(),
                None,
            )
            .await;
            return;
        }
    }
    // If lab doesn't exist yet, it will be created and owned by the authenticated user

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

    // Create UpRequest - use authenticated username
    let request = data::UpRequest {
        lab_id,
        manifest: manifest_value,
        username: auth_ctx.username.clone(),
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
                Ok(result) => {
                    tracing::info!("User '{}' brought up lab successfully", auth_ctx.username);
                    ServerMessage::RpcResponse {
                        id,
                        result: Some(result),
                        error: None,
                    }
                }
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

/// Handle "auth.login" RPC call
///
/// Expected params: {"username": "string", "password": "string"}
async fn handle_auth_login(
    id: String,
    params: serde_json::Value,
    state: &AppState,
) -> ServerMessage {
    // Parse params into LoginRequest
    let login_request: data::LoginRequest = match serde_json::from_value(params) {
        Ok(req) => req,
        Err(e) => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: -32602,
                    message: "Invalid params: expected {username: string, password: string}"
                        .to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Get user from database (for authentication)
    let user = match db::get_user_for_auth(&state.db, &login_request.username).await {
        Ok(user) => user,
        Err(_) => {
            // Don't reveal whether user exists or not
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: -32001,
                    message: "Invalid username or password".to_string(),
                    context: None,
                }),
            };
        }
    };

    // Verify password
    match shared::auth::password::verify_password(&login_request.password, &user.password_hash) {
        Ok(true) => {
            // Password is correct, create JWT token
            use shared::konst::JWT_TOKEN_EXPIRY_SECONDS;

            match crate::auth::jwt::create_token(
                &state.jwt_secret,
                &user.username,
                user.is_admin,
                JWT_TOKEN_EXPIRY_SECONDS,
            ) {
                Ok(token) => {
                    let expires_at = chrono::Utc::now().timestamp() + JWT_TOKEN_EXPIRY_SECONDS;

                    let response = data::LoginResponse {
                        token,
                        username: user.username.clone(),
                        is_admin: user.is_admin,
                        expires_at,
                    };

                    tracing::info!(
                        username = %user.username,
                        is_admin = user.is_admin,
                        "User logged in successfully"
                    );

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
                    tracing::error!(error = %e, "Failed to create JWT token");
                    ServerMessage::RpcResponse {
                        id,
                        result: None,
                        error: Some(RpcError {
                            code: -32603,
                            message: "Failed to create authentication token".to_string(),
                            context: Some(format!("{:?}", e)),
                        }),
                    }
                }
            }
        }
        Ok(false) => {
            tracing::warn!(
                username = %login_request.username,
                "Login failed: invalid password"
            );
            ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: -32001,
                    message: "Invalid username or password".to_string(),
                    context: None,
                }),
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "Password verification error");
            ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: -32603,
                    message: "Authentication error".to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            }
        }
    }
}

/// Handle "auth.validate" RPC call
///
/// Expected params: {"token": "string"}
async fn handle_auth_validate(
    id: String,
    params: serde_json::Value,
    state: &AppState,
) -> ServerMessage {
    // Parse params into ValidateRequest
    let validate_request: data::ValidateRequest = match serde_json::from_value(params) {
        Ok(req) => req,
        Err(e) => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: -32602,
                    message: "Invalid params: expected {token: string}".to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Validate token
    match crate::auth::jwt::validate_token(&state.jwt_secret, &validate_request.token) {
        Ok(claims) => {
            let response = data::ValidateResponse {
                valid: true,
                username: Some(claims.sub.clone()),
                is_admin: Some(claims.is_admin),
                expires_at: Some(claims.exp),
            };

            tracing::debug!(username = %claims.sub, "Token validated successfully");

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
        Err(_) => {
            let response = data::ValidateResponse {
                valid: false,
                username: None,
                is_admin: None,
                expires_at: None,
            };

            tracing::debug!("Token validation failed");

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
    }
}
