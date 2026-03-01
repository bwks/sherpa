use axum::extract::ws::Message;
use serde_json;
use std::sync::Arc;
use surrealdb_types::Datetime;
use tokio::sync::mpsc;

use crate::api::websocket::connection::Connection;
use crate::api::websocket::messages::{RpcError, ServerMessage};
use crate::auth::middleware;
use crate::daemon::state::AppState;
use crate::services::{
    clean, container_pull, destroy, down, import, inspect, progress, resume, up,
};
use shared::data;
use shared::error::RpcErrorCode;
use shared::konst::{
    RPC_MSG_ACCESS_DENIED_LAB, RPC_MSG_ACCESS_DENIED_LAST_ADMIN, RPC_MSG_ACCESS_DENIED_OWN_INFO,
    RPC_MSG_ACCESS_DENIED_OWN_PASSWORD, RPC_MSG_ACCESS_DENIED_SELF_DELETE,
    RPC_MSG_ADMIN_ONLY_CLEAN, RPC_MSG_ADMIN_ONLY_CONTAINER_PULL, RPC_MSG_ADMIN_ONLY_IMAGE_IMPORT,
    RPC_MSG_ADMIN_ONLY_IMAGE_SCAN, RPC_MSG_AUTH_ERROR, RPC_MSG_AUTH_INVALID, RPC_MSG_AUTH_REQUIRED,
    RPC_MSG_CONTAINER_PULL_FAILED, RPC_MSG_IMAGE_IMPORT_FAILED, RPC_MSG_IMAGE_LIST_FAILED,
    RPC_MSG_IMAGE_SCAN_FAILED, RPC_MSG_INVALID_PARAMS_CHANGE_PASSWORD,
    RPC_MSG_INVALID_PARAMS_CONTAINER_PULL, RPC_MSG_INVALID_PARAMS_CREATE_USER,
    RPC_MSG_INVALID_PARAMS_DELETE_USER, RPC_MSG_INVALID_PARAMS_GET_USER_INFO,
    RPC_MSG_INVALID_PARAMS_IMAGE_LIST, RPC_MSG_INVALID_PARAMS_IMPORT,
    RPC_MSG_INVALID_PARAMS_LAB_ID, RPC_MSG_INVALID_PARAMS_LOGIN, RPC_MSG_INVALID_PARAMS_MANIFEST,
    RPC_MSG_INVALID_PARAMS_TOKEN, RPC_MSG_LAB_CLEAN_FAILED, RPC_MSG_LAB_DESTROY_FAILED,
    RPC_MSG_LAB_DOWN_FAILED, RPC_MSG_LAB_INSPECT_FAILED, RPC_MSG_LAB_RESUME_FAILED,
    RPC_MSG_LAB_UP_FAILED, RPC_MSG_PASSWORD_VALIDATION_FAILED, RPC_MSG_SERIALIZE_FAILED,
    RPC_MSG_TOKEN_CREATE_FAILED, RPC_MSG_USER_ADMIN_ONLY_CREATE, RPC_MSG_USER_ADMIN_ONLY_DELETE,
    RPC_MSG_USER_ADMIN_ONLY_LIST, RPC_MSG_USER_CREATE_FAILED, RPC_MSG_USER_DELETE_FAILED,
    RPC_MSG_USER_DELETE_SAFETY_CHECK_FAILED, RPC_MSG_USER_LIST_FAILED,
    RPC_MSG_USER_PASSWORD_UPDATE_FAILED,
};

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
        "down" => handle_down(id, params, state).await,
        "resume" => handle_resume(id, params, state).await,
        // Note: "destroy" is handled separately via handle_streaming_rpc_request
        "clean" => handle_clean(id, params, state).await,
        "image.import" => handle_image_import(id, params, state).await,
        "image.list" => handle_image_list(id, params, state).await,
        "image.scan" => handle_image_scan(id, params, state).await,
        "image.pull" => handle_image_pull(id, params, state).await,
        "user.create" => handle_user_create(id, params, state).await,
        "user.list" => handle_user_list(id, params, state).await,
        "user.delete" => handle_user_delete(id, params, state).await,
        "user.passwd" => handle_user_passwd(id, params, state).await,
        "user.info" => handle_user_info(id, params, state).await,
        // Note: "up" is handled separately via handle_streaming_rpc_request
        _ => {
            // Unknown method
            ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::MethodNotFound,
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
        "destroy" => handle_destroy_streaming(id, params, state, connection).await,
        _ => {
            // Unknown streaming method
            let response = ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::MethodNotFound,
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
                    code: RpcErrorCode::AuthRequired,
                    message: RPC_MSG_AUTH_REQUIRED.to_string(),
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
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_LAB_ID.to_string(),
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
                        code: RpcErrorCode::AccessDenied,
                        message: RPC_MSG_ACCESS_DENIED_LAB.to_string(),
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
                    code: RpcErrorCode::NotFound,
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
                        code: RpcErrorCode::InternalError,
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
                    code: RpcErrorCode::ServerError,
                    message: RPC_MSG_LAB_INSPECT_FAILED.to_string(),
                    context: Some(error_chain),
                }),
            }
        }
    }
}

/// Handle "down" RPC call — suspend all VMs for a lab
///
/// Expected params: {"lab_id": "string", "token": "string"}
async fn handle_down(id: String, params: serde_json::Value, state: &AppState) -> ServerMessage {
    // Authenticate the request
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for down: {}", e);
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::AuthRequired,
                    message: RPC_MSG_AUTH_REQUIRED.to_string(),
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
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_LAB_ID.to_string(),
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
                    "User '{}' attempted to down lab '{}' owned by '{}'",
                    auth_ctx.username,
                    lab_id,
                    owner_username
                );
                return ServerMessage::RpcResponse {
                    id,
                    result: None,
                    error: Some(RpcError {
                        code: RpcErrorCode::AccessDenied,
                        message: RPC_MSG_ACCESS_DENIED_LAB.to_string(),
                        context: None,
                    }),
                };
            }
        }
        Err(e) => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::NotFound,
                    message: format!("Lab not found: {}", lab_id),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    }

    // Call service
    match down::suspend_lab_vms(&lab_id, state).await {
        Ok(response) => match serde_json::to_value(&response) {
            Ok(result) => {
                tracing::info!(
                    "User '{}' suspended lab '{}' VMs",
                    auth_ctx.username,
                    lab_id
                );
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
                    code: RpcErrorCode::InternalError,
                    message: RPC_MSG_SERIALIZE_FAILED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            },
        },
        Err(e) => {
            let error_chain = format!("{:?}", e);
            ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::ServerError,
                    message: RPC_MSG_LAB_DOWN_FAILED.to_string(),
                    context: Some(error_chain),
                }),
            }
        }
    }
}

/// Handle "resume" RPC call — resume all paused VMs for a lab
///
/// Expected params: {"lab_id": "string", "token": "string"}
async fn handle_resume(id: String, params: serde_json::Value, state: &AppState) -> ServerMessage {
    // Authenticate the request
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for resume: {}", e);
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::AuthRequired,
                    message: RPC_MSG_AUTH_REQUIRED.to_string(),
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
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_LAB_ID.to_string(),
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
                    "User '{}' attempted to resume lab '{}' owned by '{}'",
                    auth_ctx.username,
                    lab_id,
                    owner_username
                );
                return ServerMessage::RpcResponse {
                    id,
                    result: None,
                    error: Some(RpcError {
                        code: RpcErrorCode::AccessDenied,
                        message: RPC_MSG_ACCESS_DENIED_LAB.to_string(),
                        context: None,
                    }),
                };
            }
        }
        Err(e) => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::NotFound,
                    message: format!("Lab not found: {}", lab_id),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    }

    // Call service
    match resume::resume_lab_vms(&lab_id, state).await {
        Ok(response) => match serde_json::to_value(&response) {
            Ok(result) => {
                tracing::info!("User '{}' resumed lab '{}' VMs", auth_ctx.username, lab_id);
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
                    code: RpcErrorCode::InternalError,
                    message: RPC_MSG_SERIALIZE_FAILED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            },
        },
        Err(e) => {
            let error_chain = format!("{:?}", e);
            ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::ServerError,
                    message: RPC_MSG_LAB_RESUME_FAILED.to_string(),
                    context: Some(error_chain),
                }),
            }
        }
    }
}

/// Handle "destroy" RPC call
///
/// Expected params: {"lab_id": "string", "token": "string"}
async fn handle_destroy_streaming(
    id: String,
    params: serde_json::Value,
    state: &AppState,
    connection: &Arc<Connection>,
) {
    // Helper function to send error and return
    let send_error = |id: String, code: RpcErrorCode, message: String, context: Option<String>| async move {
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
            tracing::warn!("Authentication failed for destroy: {}", e);
            send_error(
                id,
                RpcErrorCode::AuthRequired,
                RPC_MSG_AUTH_REQUIRED.to_string(),
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
                RpcErrorCode::InvalidParams,
                RPC_MSG_INVALID_PARAMS_LAB_ID.to_string(),
                None,
            )
            .await;
            return;
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
                send_error(
                    id,
                    RpcErrorCode::AccessDenied,
                    RPC_MSG_ACCESS_DENIED_LAB.to_string(),
                    None,
                )
                .await;
                return;
            }
        }
        Err(e) => {
            send_error(
                id,
                RpcErrorCode::NotFound,
                format!("Lab not found: {}", lab_id),
                Some(format!("{:?}", e)),
            )
            .await;
            return;
        }
    }

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

    // User is authenticated and authorized - use their username from the token
    let request = data::DestroyRequest {
        lab_id,
        username: auth_ctx.username.clone(),
    };

    // Call service with progress sender
    let result = destroy::destroy_lab(request, state, progress).await;

    // Wait for forward task to complete (channel closes when progress is dropped)
    let _ = forward_task.await;

    // Send final RPC response
    let response = match result {
        Ok(destroy_response) => match serde_json::to_value(&destroy_response) {
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
                    code: RpcErrorCode::InternalError,
                    message: RPC_MSG_SERIALIZE_FAILED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            },
        },
        Err(e) => {
            let error_chain = format!("{:?}", e);
            ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::ServerError,
                    message: RPC_MSG_LAB_DESTROY_FAILED.to_string(),
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

/// Handle "clean" RPC call (admin-only)
///
/// Expected params: {"lab_id": "string", "token": "string"}
async fn handle_clean(id: String, params: serde_json::Value, state: &AppState) -> ServerMessage {
    // Authenticate the request
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for clean: {}", e);
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::AuthRequired,
                    message: RPC_MSG_AUTH_REQUIRED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Admin-only check
    if !auth_ctx.is_admin {
        tracing::warn!(
            "User '{}' attempted clean without admin privileges",
            auth_ctx.username
        );
        return ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(RpcError {
                code: RpcErrorCode::AccessDenied,
                message: RPC_MSG_ADMIN_ONLY_CLEAN.to_string(),
                context: None,
            }),
        };
    }

    // Parse params
    let lab_id = match params.get("lab_id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_LAB_ID.to_string(),
                    context: None,
                }),
            };
        }
    };

    // Call clean service
    match clean::clean_lab(&lab_id, state).await {
        Ok(response) => match serde_json::to_value(&response) {
            Ok(result) => {
                tracing::info!(
                    "Admin '{}' cleaned lab '{}' successfully",
                    auth_ctx.username,
                    lab_id
                );
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
                    code: RpcErrorCode::InternalError,
                    message: RPC_MSG_SERIALIZE_FAILED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            },
        },
        Err(e) => {
            let error_chain = format!("{:?}", e);
            ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::ServerError,
                    message: RPC_MSG_LAB_CLEAN_FAILED.to_string(),
                    context: Some(error_chain),
                }),
            }
        }
    }
}

/// Handle "image.import" RPC call (admin-only)
///
/// Expected params: ImportRequest {"model": "string", "version": "string", "src": "string", "token": "string"}
async fn handle_image_import(
    id: String,
    params: serde_json::Value,
    state: &AppState,
) -> ServerMessage {
    // Authenticate the request
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for image.import: {}", e);
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::AuthRequired,
                    message: RPC_MSG_AUTH_REQUIRED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Admin-only check
    if !auth_ctx.is_admin {
        tracing::warn!(
            "User '{}' attempted image import without admin privileges",
            auth_ctx.username
        );
        return ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(RpcError {
                code: RpcErrorCode::AccessDenied,
                message: RPC_MSG_ADMIN_ONLY_IMAGE_IMPORT.to_string(),
                context: None,
            }),
        };
    }

    // Parse params into ImportRequest
    let request: data::ImportRequest = match serde_json::from_value(params) {
        Ok(req) => req,
        Err(e) => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_IMPORT.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Call import service
    match import::import_image(request, state).await {
        Ok(response) => match serde_json::to_value(&response) {
            Ok(result) => {
                tracing::info!(
                    "Admin '{}' imported image successfully (model={}, version={})",
                    auth_ctx.username,
                    response.model,
                    response.version
                );
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
                    code: RpcErrorCode::InternalError,
                    message: RPC_MSG_SERIALIZE_FAILED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            },
        },
        Err(e) => {
            let error_chain = format!("{:?}", e);
            ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::ServerError,
                    message: RPC_MSG_IMAGE_IMPORT_FAILED.to_string(),
                    context: Some(error_chain),
                }),
            }
        }
    }
}

/// Handle "image.list" RPC call
///
/// Expected params: ListImagesRequest {"model": "string"?, "kind": "string"?, "token": "string"}
async fn handle_image_list(
    id: String,
    params: serde_json::Value,
    state: &AppState,
) -> ServerMessage {
    // Authenticate the request
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for image.list: {}", e);
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::AuthRequired,
                    message: RPC_MSG_AUTH_REQUIRED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Parse params into ListImagesRequest
    let request: data::ListImagesRequest = match serde_json::from_value(params) {
        Ok(req) => req,
        Err(e) => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_IMAGE_LIST.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Call list images service
    match import::list_images(request, state).await {
        Ok(response) => match serde_json::to_value(&response) {
            Ok(result) => {
                tracing::info!(
                    "User '{}' listed images (total={})",
                    auth_ctx.username,
                    response.total
                );
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
                    code: RpcErrorCode::InternalError,
                    message: RPC_MSG_SERIALIZE_FAILED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            },
        },
        Err(e) => {
            let error_chain = format!("{:?}", e);
            ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::ServerError,
                    message: RPC_MSG_IMAGE_LIST_FAILED.to_string(),
                    context: Some(error_chain),
                }),
            }
        }
    }
}

/// Handle "image.scan" RPC call (admin-only)
///
/// Expected params: ScanImagesRequest {"kind": "string"?, "token": "string"}
async fn handle_image_scan(
    id: String,
    params: serde_json::Value,
    state: &AppState,
) -> ServerMessage {
    // Authenticate the request
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for image.scan: {}", e);
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::AuthRequired,
                    message: RPC_MSG_AUTH_REQUIRED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Admin-only check
    if !auth_ctx.is_admin {
        tracing::warn!(
            "User '{}' attempted image scan without admin privileges",
            auth_ctx.username
        );
        return ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(RpcError {
                code: RpcErrorCode::AccessDenied,
                message: RPC_MSG_ADMIN_ONLY_IMAGE_SCAN.to_string(),
                context: None,
            }),
        };
    }

    // Parse params into ScanImagesRequest
    let request: data::ScanImagesRequest = match serde_json::from_value(params) {
        Ok(req) => req,
        Err(e) => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::InvalidParams,
                    message: "Invalid params: expected ScanImagesRequest".to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Call scan service
    match import::scan_images(request, state).await {
        Ok(response) => match serde_json::to_value(&response) {
            Ok(result) => {
                tracing::info!(
                    "Admin '{}' scanned images (found={}, imported={})",
                    auth_ctx.username,
                    response.scanned.len(),
                    response.total_imported
                );
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
                    code: RpcErrorCode::InternalError,
                    message: RPC_MSG_SERIALIZE_FAILED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            },
        },
        Err(e) => {
            let error_chain = format!("{:?}", e);
            ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::ServerError,
                    message: RPC_MSG_IMAGE_SCAN_FAILED.to_string(),
                    context: Some(error_chain),
                }),
            }
        }
    }
}

/// Handle "image.pull" RPC call
///
/// Expected params: ContainerPullRequest {"repo": "string", "tag": "string", "token": "string"}
async fn handle_image_pull(
    id: String,
    params: serde_json::Value,
    state: &AppState,
) -> ServerMessage {
    // Authenticate the request
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for image.pull: {}", e);
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::AuthRequired,
                    message: RPC_MSG_AUTH_REQUIRED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Admin-only check
    if !auth_ctx.is_admin {
        tracing::warn!(
            "User '{}' attempted container pull without admin privileges",
            auth_ctx.username
        );
        return ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(RpcError {
                code: RpcErrorCode::AccessDenied,
                message: RPC_MSG_ADMIN_ONLY_CONTAINER_PULL.to_string(),
                context: None,
            }),
        };
    }

    // Parse params into ContainerPullRequest
    let request: data::ContainerPullRequest = match serde_json::from_value(params) {
        Ok(req) => req,
        Err(e) => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_CONTAINER_PULL.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Call container pull service
    match container_pull::pull_container_image(request, state).await {
        Ok(response) => match serde_json::to_value(&response) {
            Ok(result) => {
                tracing::info!(
                    "Admin '{}' pulled container image {}:{}",
                    auth_ctx.username,
                    response.repo,
                    response.tag
                );
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
                    code: RpcErrorCode::InternalError,
                    message: RPC_MSG_SERIALIZE_FAILED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            },
        },
        Err(e) => {
            let error_chain = format!("{:?}", e);
            ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::ServerError,
                    message: RPC_MSG_CONTAINER_PULL_FAILED.to_string(),
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
    let send_error = |id: String, code: RpcErrorCode, message: String, context: Option<String>| async move {
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
                RpcErrorCode::AuthRequired,
                RPC_MSG_AUTH_REQUIRED.to_string(),
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
                RpcErrorCode::InvalidParams,
                RPC_MSG_INVALID_PARAMS_LAB_ID.to_string(),
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
                RpcErrorCode::InvalidParams,
                RPC_MSG_INVALID_PARAMS_MANIFEST.to_string(),
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
                RpcErrorCode::AccessDenied,
                RPC_MSG_ACCESS_DENIED_LAB.to_string(),
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
                        code: RpcErrorCode::InternalError,
                        message: RPC_MSG_SERIALIZE_FAILED.to_string(),
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
                    code: RpcErrorCode::ServerError,
                    message: RPC_MSG_LAB_UP_FAILED.to_string(),
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
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_LOGIN.to_string(),
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
                    code: RpcErrorCode::AuthInvalid,
                    message: RPC_MSG_AUTH_INVALID.to_string(),
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
                    let expires_at = jiff::Timestamp::now().as_second() + JWT_TOKEN_EXPIRY_SECONDS;

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
                                code: RpcErrorCode::InternalError,
                                message: RPC_MSG_SERIALIZE_FAILED.to_string(),
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
                            code: RpcErrorCode::InternalError,
                            message: RPC_MSG_TOKEN_CREATE_FAILED.to_string(),
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
                    code: RpcErrorCode::AuthInvalid,
                    message: RPC_MSG_AUTH_INVALID.to_string(),
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
                    code: RpcErrorCode::InternalError,
                    message: RPC_MSG_AUTH_ERROR.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            }
        }
    }
}

/// Handle "user.create" RPC call
///
/// Expected params: CreateUserRequest {"username": "string", "password": "string", "is_admin": bool, "ssh_keys": [string], "token": "string"}
async fn handle_user_create(
    id: String,
    params: serde_json::Value,
    state: &AppState,
) -> ServerMessage {
    // Authenticate the request
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for user.create: {}", e);
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::AuthRequired,
                    message: RPC_MSG_AUTH_REQUIRED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Check admin status
    if !auth_ctx.is_admin {
        tracing::warn!(
            "User '{}' attempted to create user without admin privileges",
            auth_ctx.username
        );
        return ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(RpcError {
                code: RpcErrorCode::AccessDenied,
                message: RPC_MSG_USER_ADMIN_ONLY_CREATE.to_string(),
                context: None,
            }),
        };
    }

    // Parse params into CreateUserRequest
    let request: data::CreateUserRequest = match serde_json::from_value(params) {
        Ok(req) => req,
        Err(e) => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_CREATE_USER.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Create user
    match db::create_user(
        &state.db,
        request.username.clone(),
        &request.password,
        request.is_admin,
        request.ssh_keys.unwrap_or_default(),
    )
    .await
    {
        Ok(user) => {
            tracing::info!(
                admin = %auth_ctx.username,
                new_user = %user.username,
                is_admin = user.is_admin,
                "User created successfully"
            );

            let response = data::CreateUserResponse {
                success: true,
                username: user.username,
                is_admin: user.is_admin,
            };

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
                        code: RpcErrorCode::InternalError,
                        message: RPC_MSG_SERIALIZE_FAILED.to_string(),
                        context: Some(format!("{:?}", e)),
                    }),
                },
            }
        }
        Err(e) => {
            tracing::error!(
                admin = %auth_ctx.username,
                username = %request.username,
                error = %e,
                "Failed to create user"
            );
            ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::ServerError,
                    message: RPC_MSG_USER_CREATE_FAILED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            }
        }
    }
}

/// Handle "user.list" RPC call
///
/// Expected params: ListUsersRequest {"token": "string"}
async fn handle_user_list(
    id: String,
    params: serde_json::Value,
    state: &AppState,
) -> ServerMessage {
    // Authenticate the request
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for user.list: {}", e);
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::AuthRequired,
                    message: RPC_MSG_AUTH_REQUIRED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Check admin status
    if !auth_ctx.is_admin {
        tracing::warn!(
            "User '{}' attempted to list users without admin privileges",
            auth_ctx.username
        );
        return ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(RpcError {
                code: RpcErrorCode::AccessDenied,
                message: RPC_MSG_USER_ADMIN_ONLY_LIST.to_string(),
                context: None,
            }),
        };
    }

    // List users
    match db::list_users(&state.db).await {
        Ok(users) => {
            // Convert to UserInfo (strip sensitive data)
            let user_list: Vec<data::UserInfo> = users
                .into_iter()
                .map(|u| data::UserInfo {
                    username: u.username,
                    is_admin: u.is_admin,
                    ssh_keys: u.ssh_keys,
                    created_at: u.created_at.timestamp(),
                    updated_at: u.updated_at.timestamp(),
                })
                .collect();

            tracing::info!(
                admin = %auth_ctx.username,
                count = user_list.len(),
                "Listed users successfully"
            );

            let response = data::ListUsersResponse { users: user_list };

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
                        code: RpcErrorCode::InternalError,
                        message: RPC_MSG_SERIALIZE_FAILED.to_string(),
                        context: Some(format!("{:?}", e)),
                    }),
                },
            }
        }
        Err(e) => {
            tracing::error!(
                admin = %auth_ctx.username,
                error = %e,
                "Failed to list users"
            );
            ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::ServerError,
                    message: RPC_MSG_USER_LIST_FAILED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            }
        }
    }
}

/// Handle "user.delete" RPC call
///
/// Expected params: DeleteUserRequest {"username": "string", "token": "string"}
async fn handle_user_delete(
    id: String,
    params: serde_json::Value,
    state: &AppState,
) -> ServerMessage {
    // Authenticate the request
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for user.delete: {}", e);
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::AuthRequired,
                    message: RPC_MSG_AUTH_REQUIRED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Check admin status
    if !auth_ctx.is_admin {
        tracing::warn!(
            "User '{}' attempted to delete user without admin privileges",
            auth_ctx.username
        );
        return ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(RpcError {
                code: RpcErrorCode::AccessDenied,
                message: RPC_MSG_USER_ADMIN_ONLY_DELETE.to_string(),
                context: None,
            }),
        };
    }

    // Parse params into DeleteUserRequest
    let request: data::DeleteUserRequest = match serde_json::from_value(params) {
        Ok(req) => req,
        Err(e) => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_DELETE_USER.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Prevent self-deletion
    if request.username == auth_ctx.username {
        tracing::warn!(
            "User '{}' attempted to delete themselves",
            auth_ctx.username
        );
        return ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(RpcError {
                code: RpcErrorCode::AccessDenied,
                message: RPC_MSG_ACCESS_DENIED_SELF_DELETE.to_string(),
                context: None,
            }),
        };
    }

    // Check if this is the last admin
    match db::list_users(&state.db).await {
        Ok(users) => {
            let admin_count = users.iter().filter(|u| u.is_admin).count();

            // Get the user to be deleted
            let user_to_delete = match users.iter().find(|u| u.username == request.username) {
                Some(u) => u,
                None => {
                    return ServerMessage::RpcResponse {
                        id,
                        result: None,
                        error: Some(RpcError {
                            code: RpcErrorCode::NotFound,
                            message: format!("User not found: {}", request.username),
                            context: None,
                        }),
                    };
                }
            };

            // If deleting an admin and this is the last admin, prevent deletion
            if user_to_delete.is_admin && admin_count <= 1 {
                tracing::warn!(
                    admin = %auth_ctx.username,
                    target = %request.username,
                    "Attempted to delete last admin user"
                );
                return ServerMessage::RpcResponse {
                    id,
                    result: None,
                    error: Some(RpcError {
                        code: RpcErrorCode::AccessDenied,
                        message: RPC_MSG_ACCESS_DENIED_LAST_ADMIN.to_string(),
                        context: None,
                    }),
                };
            }
        }
        Err(e) => {
            tracing::error!(
                admin = %auth_ctx.username,
                error = %e,
                "Failed to check admin count before deletion"
            );
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::ServerError,
                    message: RPC_MSG_USER_DELETE_SAFETY_CHECK_FAILED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    }

    // Delete user (cascade delete labs)
    match db::delete_user_by_username(&state.db, &request.username).await {
        Ok(_) => {
            tracing::info!(
                admin = %auth_ctx.username,
                deleted_user = %request.username,
                "User deleted successfully"
            );

            let response = data::DeleteUserResponse {
                success: true,
                username: request.username,
            };

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
                        code: RpcErrorCode::InternalError,
                        message: RPC_MSG_SERIALIZE_FAILED.to_string(),
                        context: Some(format!("{:?}", e)),
                    }),
                },
            }
        }
        Err(e) => {
            tracing::error!(
                admin = %auth_ctx.username,
                username = %request.username,
                error = %e,
                "Failed to delete user"
            );
            ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::ServerError,
                    message: RPC_MSG_USER_DELETE_FAILED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            }
        }
    }
}

/// Handle "user.passwd" RPC call
///
/// Expected params: ChangePasswordRequest {"username": "string", "new_password": "string", "token": "string"}
async fn handle_user_passwd(
    id: String,
    params: serde_json::Value,
    state: &AppState,
) -> ServerMessage {
    // Authenticate the request
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for user.passwd: {}", e);
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::AuthRequired,
                    message: RPC_MSG_AUTH_REQUIRED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Parse params into ChangePasswordRequest
    let request: data::ChangePasswordRequest = match serde_json::from_value(params) {
        Ok(req) => req,
        Err(e) => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_CHANGE_PASSWORD.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Check authorization: admin can change any password, regular user only their own
    if !auth_ctx.is_admin && request.username != auth_ctx.username {
        tracing::warn!(
            user = %auth_ctx.username,
            target = %request.username,
            "User attempted to change another user's password without admin privileges"
        );
        return ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(RpcError {
                code: RpcErrorCode::AccessDenied,
                message: RPC_MSG_ACCESS_DENIED_OWN_PASSWORD.to_string(),
                context: None,
            }),
        };
    }

    // Get the user to update
    let mut user = match db::get_user(&state.db, &request.username).await {
        Ok(u) => u,
        Err(_) => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::NotFound,
                    message: format!("User not found: {}", request.username),
                    context: None,
                }),
            };
        }
    };

    // Hash the new password (this also validates password strength)
    let new_password_hash = match shared::auth::password::hash_password(&request.new_password) {
        Ok(hash) => hash,
        Err(e) => {
            tracing::warn!(
                user = %auth_ctx.username,
                target = %request.username,
                error = %e,
                "Password validation failed"
            );
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_PASSWORD_VALIDATION_FAILED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Update password and updated_at timestamp
    user.password_hash = new_password_hash;
    user.updated_at = Datetime::default();

    match db::update_user(&state.db, user).await {
        Ok(_) => {
            tracing::info!(
                user = %auth_ctx.username,
                target = %request.username,
                is_admin = auth_ctx.is_admin,
                "Password changed successfully"
            );

            let response = data::ChangePasswordResponse {
                success: true,
                username: request.username,
            };

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
                        code: RpcErrorCode::InternalError,
                        message: RPC_MSG_SERIALIZE_FAILED.to_string(),
                        context: Some(format!("{:?}", e)),
                    }),
                },
            }
        }
        Err(e) => {
            tracing::error!(
                user = %auth_ctx.username,
                target = %request.username,
                error = %e,
                "Failed to update password"
            );
            ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::ServerError,
                    message: RPC_MSG_USER_PASSWORD_UPDATE_FAILED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            }
        }
    }
}

/// Handle "user.info" RPC call
///
/// Expected params: GetUserInfoRequest {"username": "string", "token": "string"}
async fn handle_user_info(
    id: String,
    params: serde_json::Value,
    state: &AppState,
) -> ServerMessage {
    // Authenticate the request
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for user.info: {}", e);
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::AuthRequired,
                    message: RPC_MSG_AUTH_REQUIRED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Parse params into GetUserInfoRequest
    let request: data::GetUserInfoRequest = match serde_json::from_value(params) {
        Ok(req) => req,
        Err(e) => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_GET_USER_INFO.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Check authorization: admin can view any user, regular user only themselves
    if !auth_ctx.is_admin && request.username != auth_ctx.username {
        tracing::warn!(
            user = %auth_ctx.username,
            target = %request.username,
            "User attempted to view another user's info without admin privileges"
        );
        return ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(RpcError {
                code: RpcErrorCode::AccessDenied,
                message: RPC_MSG_ACCESS_DENIED_OWN_INFO.to_string(),
                context: None,
            }),
        };
    }

    // Get user info
    match db::get_user(&state.db, &request.username).await {
        Ok(user) => {
            tracing::info!(
                requester = %auth_ctx.username,
                target = %request.username,
                "Retrieved user info successfully"
            );

            let user_info = data::UserInfo {
                username: user.username,
                is_admin: user.is_admin,
                ssh_keys: user.ssh_keys,
                created_at: user.created_at.timestamp(),
                updated_at: user.updated_at.timestamp(),
            };

            let response = data::GetUserInfoResponse { user: user_info };

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
                        code: RpcErrorCode::InternalError,
                        message: RPC_MSG_SERIALIZE_FAILED.to_string(),
                        context: Some(format!("{:?}", e)),
                    }),
                },
            }
        }
        Err(_) => {
            tracing::warn!(
                requester = %auth_ctx.username,
                target = %request.username,
                "User not found"
            );
            ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::NotFound,
                    message: format!("User not found: {}", request.username),
                    context: None,
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
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_TOKEN.to_string(),
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
                        code: RpcErrorCode::InternalError,
                        message: RPC_MSG_SERIALIZE_FAILED.to_string(),
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
                        code: RpcErrorCode::InternalError,
                        message: RPC_MSG_SERIALIZE_FAILED.to_string(),
                        context: Some(format!("{:?}", e)),
                    }),
                },
            }
        }
    }
}
