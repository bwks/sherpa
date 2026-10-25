use axum::extract::ws::Message;
use opentelemetry::KeyValue;
use serde_json;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;

use crate::api::websocket::connection::Connection;
use crate::api::websocket::messages::{RpcError, ServerMessage};
use crate::auth::context::AuthContext;
use crate::auth::middleware;
use crate::daemon::state::AppState;
use crate::services::error::ServiceError;
use crate::services::{
    cancel, clean, container_pull, delete, destroy, down, download, impairment, import, inspect,
    list_labs, node_secrets, progress, redeploy, resume, up, user_delete, user_ssh_keys,
};
use shared::auth::password;
use shared::data;
use shared::error::RpcErrorCode;
use shared::konst::{
    JWT_TOKEN_EXPIRY_SECONDS, RPC_MSG_ACCESS_DENIED_OWN_INFO, RPC_MSG_ACCESS_DENIED_OWN_PASSWORD,
    RPC_MSG_ADMIN_ONLY_CLEAN, RPC_MSG_ADMIN_ONLY_CONTAINER_PULL, RPC_MSG_ADMIN_ONLY_IMAGE_DELETE,
    RPC_MSG_ADMIN_ONLY_IMAGE_DOWNLOAD, RPC_MSG_ADMIN_ONLY_IMAGE_IMPORT,
    RPC_MSG_ADMIN_ONLY_IMAGE_SCAN, RPC_MSG_ADMIN_ONLY_IMAGE_SET_DEFAULT, RPC_MSG_AUTH_ERROR,
    RPC_MSG_AUTH_INVALID, RPC_MSG_AUTH_REQUIRED, RPC_MSG_CONTAINER_PULL_FAILED,
    RPC_MSG_IMAGE_DELETE_FAILED, RPC_MSG_IMAGE_DOWNLOAD_FAILED, RPC_MSG_IMAGE_IMPORT_FAILED,
    RPC_MSG_IMAGE_LIST_FAILED, RPC_MSG_IMAGE_SCAN_FAILED, RPC_MSG_IMAGE_SET_DEFAULT_FAILED,
    RPC_MSG_IMAGE_SHOW_FAILED, RPC_MSG_IMPAIRMENT_UPDATE_FAILED,
    RPC_MSG_INVALID_PARAMS_ADD_SSH_KEY, RPC_MSG_INVALID_PARAMS_CHANGE_PASSWORD,
    RPC_MSG_INVALID_PARAMS_CONTAINER_PULL, RPC_MSG_INVALID_PARAMS_CREATE_USER,
    RPC_MSG_INVALID_PARAMS_DELETE_USER, RPC_MSG_INVALID_PARAMS_GET_USER_INFO,
    RPC_MSG_INVALID_PARAMS_IMAGE_DELETE, RPC_MSG_INVALID_PARAMS_IMAGE_DOWNLOAD,
    RPC_MSG_INVALID_PARAMS_IMAGE_LIST, RPC_MSG_INVALID_PARAMS_IMAGE_SET_DEFAULT,
    RPC_MSG_INVALID_PARAMS_IMAGE_SHOW, RPC_MSG_INVALID_PARAMS_IMPAIRMENT,
    RPC_MSG_INVALID_PARAMS_IMPORT, RPC_MSG_INVALID_PARAMS_LAB_ID, RPC_MSG_INVALID_PARAMS_LOGIN,
    RPC_MSG_INVALID_PARAMS_MANIFEST, RPC_MSG_INVALID_PARAMS_NODE_SECRETS,
    RPC_MSG_INVALID_PARAMS_REDEPLOY, RPC_MSG_INVALID_PARAMS_REMOVE_SSH_KEY,
    RPC_MSG_INVALID_PARAMS_TOKEN, RPC_MSG_INVALID_TAILSCALE_KEY, RPC_MSG_LAB_CANCEL_FAILED,
    RPC_MSG_LAB_CLEAN_FAILED, RPC_MSG_LAB_DESTROY_FAILED, RPC_MSG_LAB_DOWN_FAILED,
    RPC_MSG_LAB_INSPECT_FAILED, RPC_MSG_LAB_RESUME_FAILED, RPC_MSG_LAB_UP_FAILED,
    RPC_MSG_NODE_SECRETS_FAILED, RPC_MSG_PASSWORD_VALIDATION_FAILED, RPC_MSG_REDEPLOY_FAILED,
    RPC_MSG_SERIALIZE_FAILED, RPC_MSG_SSH_KEY_ADD_FAILED, RPC_MSG_SSH_KEY_REMOVE_FAILED,
    RPC_MSG_TOKEN_CREATE_FAILED, RPC_MSG_USER_ADMIN_ONLY_CREATE, RPC_MSG_USER_ADMIN_ONLY_DELETE,
    RPC_MSG_USER_ADMIN_ONLY_LIST, RPC_MSG_USER_CREATE_FAILED, RPC_MSG_USER_DELETE_FAILED,
    RPC_MSG_USER_LIST_FAILED, RPC_MSG_USER_PASSWORD_UPDATE_FAILED,
};
use shared::util::normalize_username;

/// Authenticate request and verify admin privileges.
///
/// Returns `Ok(AuthContext)` on success, or `Err(ServerMessage)` with the appropriate
/// auth-required or access-denied error response.
async fn require_admin(
    id: &str,
    params: &serde_json::Value,
    state: &AppState,
    deny_msg: &str,
) -> Result<AuthContext, ServerMessage> {
    let auth_ctx = match middleware::authenticate_request(params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed: {}", e);
            return Err(ServerMessage::RpcResponse {
                id: id.to_string(),
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::AuthRequired,
                    message: RPC_MSG_AUTH_REQUIRED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            });
        }
    };

    if !auth_ctx.is_admin {
        tracing::warn!(
            "User '{}' attempted admin-only operation without admin privileges",
            auth_ctx.username
        );
        return Err(ServerMessage::RpcResponse {
            id: id.to_string(),
            result: None,
            error: Some(RpcError {
                code: RpcErrorCode::AccessDenied,
                message: deny_msg.to_string(),
                context: None,
            }),
        });
    }

    Ok(auth_ctx)
}

/// Build the RPC error for a failed service call.
///
/// Authorization failures from the service's policy checks keep their own codes so clients
/// can tell "not allowed" and "does not exist" apart from operational failures.
fn service_error(error: &anyhow::Error, failure_message: &str) -> RpcError {
    match ServiceError::find(error) {
        Some(access @ ServiceError::Denied(_)) => RpcError {
            code: RpcErrorCode::AccessDenied,
            message: access.to_string(),
            context: None,
        },
        Some(ServiceError::NotFound(message)) => RpcError {
            code: RpcErrorCode::NotFound,
            message: message.clone(),
            context: None,
        },
        // JSON-RPC has no conflict code; the message tells the caller what to change.
        Some(ServiceError::Conflict(message)) => RpcError {
            code: RpcErrorCode::ServerError,
            message: message.clone(),
            context: None,
        },
        None => RpcError {
            code: RpcErrorCode::ServerError,
            message: failure_message.to_string(),
            context: Some(format!("{:?}", error)),
        },
    }
}

/// Authenticate request and verify admin privileges for streaming handlers.
///
/// Returns `Ok(AuthContext)` on success, or sends the error via WebSocket and returns `Err(())`.
async fn require_admin_streaming(
    id: &str,
    params: &serde_json::Value,
    state: &AppState,
    connection: &Arc<Connection>,
    deny_msg: &str,
) -> Result<AuthContext, ()> {
    let auth_ctx = match middleware::authenticate_request(params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed: {}", e);
            let response = ServerMessage::RpcResponse {
                id: id.to_string(),
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::AuthRequired,
                    message: RPC_MSG_AUTH_REQUIRED.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
            if let Ok(json) = serde_json::to_string(&response) {
                let _ = connection.send(Message::Text(json.into())).await;
            }
            return Err(());
        }
    };

    if !auth_ctx.is_admin {
        tracing::warn!(
            "User '{}' attempted admin-only operation without admin privileges",
            auth_ctx.username
        );
        let response = ServerMessage::RpcResponse {
            id: id.to_string(),
            result: None,
            error: Some(RpcError {
                code: RpcErrorCode::AccessDenied,
                message: deny_msg.to_string(),
                context: None,
            }),
        };
        if let Ok(json) = serde_json::to_string(&response) {
            let _ = connection.send(Message::Text(json.into())).await;
        }
        return Err(());
    }

    Ok(auth_ctx)
}

/// Send an RPC error response over the WebSocket connection.
async fn send_rpc_error(
    connection: &Arc<Connection>,
    id: String,
    code: RpcErrorCode,
    message: String,
    context: Option<String>,
) {
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
}

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
    let start = Instant::now();
    let method_attr = KeyValue::new("rpc.method", method.clone());

    let response = match method.as_str() {
        "auth.login" => handle_auth_login(id, params, state).await,
        "auth.validate" => handle_auth_validate(id, params, state).await,
        "inspect" => handle_inspect(id, params, state).await,
        "download" => handle_download(id, params, state).await,
        "labs.list" => handle_labs_list(id, params, state).await,
        "down" => handle_down(id, params, state).await,
        "node.secrets" => handle_node_secrets(id, params, state).await,
        "cancel" => handle_cancel(id, params, state).await,
        "link.update_impairment" => handle_link_update_impairment(id, params, state).await,
        // Note: "resume" is handled separately via handle_streaming_rpc_request
        // Note: "destroy" is handled separately via handle_streaming_rpc_request
        "clean" => match require_admin(&id, &params, state, RPC_MSG_ADMIN_ONLY_CLEAN).await {
            Ok(auth_ctx) => handle_clean(id, params, state, auth_ctx).await,
            Err(e) => e,
        },
        // Note: "image.import" is handled separately via handle_streaming_rpc_request
        "image.delete" => {
            match require_admin(&id, &params, state, RPC_MSG_ADMIN_ONLY_IMAGE_DELETE).await {
                Ok(auth_ctx) => handle_image_delete(id, params, state, auth_ctx).await,
                Err(e) => e,
            }
        }
        "image.set_default" => {
            match require_admin(&id, &params, state, RPC_MSG_ADMIN_ONLY_IMAGE_SET_DEFAULT).await {
                Ok(auth_ctx) => handle_image_set_default(id, params, state, auth_ctx).await,
                Err(e) => e,
            }
        }
        "image.list" => handle_image_list(id, params, state).await,
        "image.show" => handle_image_show(id, params, state).await,
        "image.scan" => {
            match require_admin(&id, &params, state, RPC_MSG_ADMIN_ONLY_IMAGE_SCAN).await {
                Ok(auth_ctx) => handle_image_scan(id, params, state, auth_ctx).await,
                Err(e) => e,
            }
        }
        // Note: "image.pull" is handled separately via handle_streaming_rpc_request
        // Note: "image.download" is handled separately via handle_streaming_rpc_request
        "user.create" => {
            match require_admin(&id, &params, state, RPC_MSG_USER_ADMIN_ONLY_CREATE).await {
                Ok(auth_ctx) => handle_user_create(id, params, state, auth_ctx).await,
                Err(e) => e,
            }
        }
        "user.list" => match require_admin(&id, &params, state, RPC_MSG_USER_ADMIN_ONLY_LIST).await
        {
            Ok(auth_ctx) => handle_user_list(id, params, state, auth_ctx).await,
            Err(e) => e,
        },
        "user.delete" => {
            match require_admin(&id, &params, state, RPC_MSG_USER_ADMIN_ONLY_DELETE).await {
                Ok(auth_ctx) => handle_user_delete(id, params, state, auth_ctx).await,
                Err(e) => e,
            }
        }
        "user.passwd" => handle_user_passwd(id, params, state).await,
        "user.info" => handle_user_info(id, params, state).await,
        "user.ssh_key.add" => handle_user_ssh_key_add(id, params, state).await,
        "user.ssh_key.remove" => handle_user_ssh_key_remove(id, params, state).await,
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
    };

    state
        .metrics
        .rpc_duration
        .record(start.elapsed().as_secs_f64(), &[method_attr]);

    response
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
    let start = Instant::now();
    let method_attr = KeyValue::new("rpc.method", method.clone());

    match method.as_str() {
        "up" => handle_up(id, params, state, connection).await,
        "destroy" => handle_destroy_streaming(id, params, state, connection).await,
        "resume" => handle_resume_streaming(id, params, state, connection).await,
        "redeploy" => handle_redeploy_streaming(id, params, state, connection).await,
        "image.import" => {
            if let Ok(auth_ctx) = require_admin_streaming(
                &id,
                &params,
                state,
                connection,
                RPC_MSG_ADMIN_ONLY_IMAGE_IMPORT,
            )
            .await
            {
                handle_image_import_streaming(id, params, state, connection, auth_ctx).await;
            }
        }
        "image.pull" => {
            if let Ok(auth_ctx) = require_admin_streaming(
                &id,
                &params,
                state,
                connection,
                RPC_MSG_ADMIN_ONLY_CONTAINER_PULL,
            )
            .await
            {
                handle_image_pull_streaming(id, params, state, connection, auth_ctx).await;
            }
        }
        "image.download" => {
            if let Ok(auth_ctx) = require_admin_streaming(
                &id,
                &params,
                state,
                connection,
                RPC_MSG_ADMIN_ONLY_IMAGE_DOWNLOAD,
            )
            .await
            {
                handle_image_download_streaming(id, params, state, connection, auth_ctx).await;
            }
        }
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

    state
        .metrics
        .rpc_duration
        .record(start.elapsed().as_secs_f64(), &[method_attr]);
}

/// Handle "labs.list" RPC call.
///
/// Expected params: {"token": "string"}
async fn handle_labs_list(
    id: String,
    params: serde_json::Value,
    state: &AppState,
) -> ServerMessage {
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for labs.list: {}", e);
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

    match list_labs::list_labs(&auth_ctx, None, state).await {
        Ok(response) => match serde_json::to_value(&response) {
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
        },
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, "Failed to list labs")),
        },
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

    // The service enforces owner-or-admin access
    match inspect::inspect_lab(&auth_ctx, &lab_id, state).await {
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
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_LAB_INSPECT_FAILED)),
        },
    }
}

/// Handle "download" RPC call — return lab files for CLI use
///
/// Expected params: {"lab_id": "string", "token": "string"}
async fn handle_download(id: String, params: serde_json::Value, state: &AppState) -> ServerMessage {
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for download: {}", e);
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

    match download::download_lab_files(&auth_ctx, &lab_id, state).await {
        Ok(response) => match serde_json::to_value(&response) {
            Ok(result) => {
                tracing::info!(
                    "User '{}' downloaded lab files for '{}'",
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
                    message: "Failed to serialize response".to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            },
        },
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, "Failed to download lab files")),
        },
    }
}

/// Handle "down" RPC call — shutdown all (or a specific) node(s) for a lab
///
/// Expected params: {"lab_id": "string", "token": "string", "node_name": "string" (optional)}
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

    // Parse optional node_name
    let node_name = params
        .get("node_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Call service
    match down::shutdown_lab_nodes(&auth_ctx, &lab_id, node_name.as_deref(), state).await {
        Ok(response) => match serde_json::to_value(&response) {
            Ok(result) => {
                tracing::info!(
                    "User '{}' shut down lab '{}' nodes (node_name: {:?})",
                    auth_ctx.username,
                    lab_id,
                    node_name
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
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_LAB_DOWN_FAILED)),
        },
    }
}

/// Handle "node.secrets" RPC call — rewrite a container node's secret files
///
/// Expected params: NodeSecretsRequest and "token". The files are never logged.
async fn handle_node_secrets(
    id: String,
    params: serde_json::Value,
    state: &AppState,
) -> ServerMessage {
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for node.secrets: {}", e);
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

    // The parse error is not returned, as it could quote secret content.
    let request: data::NodeSecretsRequest = match serde_json::from_value(params) {
        Ok(request) => request,
        Err(_) => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_NODE_SECRETS.to_string(),
                    context: None,
                }),
            };
        }
    };

    match node_secrets::update_node_secrets(&auth_ctx, request, state).await {
        Ok(response) => match serde_json::to_value(&response) {
            Ok(result) => {
                tracing::info!(
                    "User '{}' rewrote {} secret files for node '{}'",
                    auth_ctx.username,
                    response.files_written,
                    response.node_name
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
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_NODE_SECRETS_FAILED)),
        },
    }
}

/// Handle "cancel" RPC call — stop the operations running on a lab and wait for them to end
///
/// Expected params: {"lab_id": "string", "token": "string"}
async fn handle_cancel(id: String, params: serde_json::Value, state: &AppState) -> ServerMessage {
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for cancel: {}", e);
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

    match cancel::cancel_lab_operations(&auth_ctx, &lab_id, state).await {
        Ok(response) => match serde_json::to_value(&response) {
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
        },
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_LAB_CANCEL_FAILED)),
        },
    }
}

/// Handle "resume" streaming RPC call: report readiness progress, then send the response.
///
/// Expected params: {"lab_id": "string", "token": "string", "node_name": "string" (optional)}
async fn handle_resume_streaming(
    id: String,
    params: serde_json::Value,
    state: &AppState,
    connection: &Arc<Connection>,
) {
    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel();
    let conn_clone = Arc::clone(connection);
    let forward_task = tokio::spawn(async move {
        while let Some(msg) = progress_rx.recv().await {
            let _ = conn_clone.send(msg).await;
        }
    });
    let progress = progress::ProgressSender::new(progress_tx);

    let response = handle_resume(id, params, state, &progress).await;

    // Close the channel so the forward task drains and finishes before the response.
    drop(progress);
    let _ = forward_task.await;
    if let Ok(json) = serde_json::to_string(&response) {
        let _ = connection.send(Message::Text(json.into())).await;
    }
}

/// The optional `tailscale_auth_key` param of `up` and `resume`.
fn tailscale_auth_key_param(
    params: &serde_json::Value,
) -> Result<Option<data::TailnetAuthKey>, serde_json::Error> {
    match params.get("tailscale_auth_key") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone()).map(Some),
    }
}

/// Handle "resume" RPC call — start/poweron all (or a specific) node(s) for a lab
async fn handle_resume(
    id: String,
    params: serde_json::Value,
    state: &AppState,
    progress: &progress::ProgressSender,
) -> ServerMessage {
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

    // Parse optional node_name
    let node_name = params
        .get("node_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let Ok(tailscale_auth_key) = tailscale_auth_key_param(&params) else {
        return ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(RpcError {
                code: RpcErrorCode::InvalidParams,
                message: RPC_MSG_INVALID_TAILSCALE_KEY.to_string(),
                context: None,
            }),
        };
    };

    // Call service
    match resume::start_lab_nodes(
        &auth_ctx,
        &lab_id,
        node_name.as_deref(),
        tailscale_auth_key.as_ref(),
        state,
        Some(progress),
    )
    .await
    {
        Ok(response) => match serde_json::to_value(&response) {
            Ok(result) => {
                tracing::info!(
                    "User '{}' started lab '{}' nodes (node_name: {:?})",
                    auth_ctx.username,
                    lab_id,
                    node_name
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
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_LAB_RESUME_FAILED)),
        },
    }
}

/// Handle "link.update_impairment" RPC call — update link impairment on a running P2p link
///
/// Expected params: {"lab_id": "string", "link_index": number, "token": "string",
///   "delay": number, "jitter": number, "loss_percent": number,
///   "reorder_percent": number, "corrupt_percent": number}
async fn handle_link_update_impairment(
    id: String,
    params: serde_json::Value,
    state: &AppState,
) -> ServerMessage {
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for link.update_impairment: {}", e);
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

    let link_index = match params.get("link_index").and_then(|v| v.as_u64()) {
        Some(idx) => idx as u16,
        None => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_IMPAIRMENT.to_string(),
                    context: None,
                }),
            };
        }
    };

    let request = data::UpdateImpairmentRequest {
        lab_id,
        link_index,
        delay: params.get("delay").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        jitter: params.get("jitter").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        loss_percent: params
            .get("loss_percent")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as f32,
        reorder_percent: params
            .get("reorder_percent")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as f32,
        corrupt_percent: params
            .get("corrupt_percent")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as f32,
    };

    match impairment::update_impairment(&auth_ctx, request, state).await {
        Ok(response) => match serde_json::to_value(&response) {
            Ok(result) => {
                tracing::info!(
                    "User '{}' updated link impairment on link {} in lab",
                    auth_ctx.username,
                    link_index,
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
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_IMPAIRMENT_UPDATE_FAILED)),
        },
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
    // Authenticate the request
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for destroy: {}", e);
            send_rpc_error(
                connection,
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
            send_rpc_error(
                connection,
                id,
                RpcErrorCode::InvalidParams,
                RPC_MSG_INVALID_PARAMS_LAB_ID.to_string(),
                None,
            )
            .await;
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

    // The service enforces owner-or-admin access before sending any progress
    let result = destroy::destroy_lab(&auth_ctx, &lab_id, state, progress).await;

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
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_LAB_DESTROY_FAILED)),
        },
    };

    // Send final response
    if let Ok(json) = serde_json::to_string(&response) {
        let _ = connection.send(Message::Text(json.into())).await;
    }
}

/// Handle "redeploy" streaming RPC call
///
/// Expected params: {"lab_id": "string", "node_name": "string", "manifest": {...}, "token": "string"}
async fn handle_redeploy_streaming(
    id: String,
    params: serde_json::Value,
    state: &AppState,
    connection: &Arc<Connection>,
) {
    // Authenticate the request
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for redeploy: {}", e);
            send_rpc_error(
                connection,
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
            send_rpc_error(
                connection,
                id,
                RpcErrorCode::InvalidParams,
                RPC_MSG_INVALID_PARAMS_REDEPLOY.to_string(),
                None,
            )
            .await;
            return;
        }
    };

    let node_name = match params.get("node_name").and_then(|v| v.as_str()) {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => {
            send_rpc_error(
                connection,
                id,
                RpcErrorCode::InvalidParams,
                RPC_MSG_INVALID_PARAMS_REDEPLOY.to_string(),
                None,
            )
            .await;
            return;
        }
    };

    let manifest = match params.get("manifest") {
        Some(m) => m.clone(),
        None => {
            send_rpc_error(
                connection,
                id,
                RpcErrorCode::InvalidParams,
                RPC_MSG_INVALID_PARAMS_REDEPLOY.to_string(),
                None,
            )
            .await;
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

    let secrets = match params.get("secrets") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => match serde_json::from_value(value.clone()) {
            Ok(secrets) => Some(secrets),
            Err(_) => {
                send_rpc_error(
                    connection,
                    id,
                    RpcErrorCode::InvalidParams,
                    "Invalid secrets field".to_string(),
                    None,
                )
                .await;
                return;
            }
        },
    };

    let request = data::RedeployRequest {
        lab_id,
        node_name: node_name.clone(),
        manifest,
        secrets,
    };

    // Call service with progress sender
    let result = redeploy::redeploy_node(&auth_ctx, request, state, progress).await;

    // Wait for forward task to complete (channel closes when progress is dropped)
    let _ = forward_task.await;

    // Send final RPC response
    let response = match result {
        Ok(redeploy_response) => match serde_json::to_value(&redeploy_response) {
            Ok(result) => {
                tracing::info!(
                    "User '{}' redeployed node '{}' successfully",
                    auth_ctx.username,
                    node_name
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
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_REDEPLOY_FAILED)),
        },
    };

    // Send final response
    if let Ok(json) = serde_json::to_string(&response) {
        let _ = connection.send(Message::Text(json.into())).await;
    }
}

/// Handle "clean" RPC call (admin-only)
///
/// Expected params: {"lab_id": "string", "token": "string"}
async fn handle_clean(
    id: String,
    params: serde_json::Value,
    state: &AppState,
    auth_ctx: AuthContext,
) -> ServerMessage {
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
    match clean::clean_lab(&auth_ctx, &lab_id, state).await {
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
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_LAB_CLEAN_FAILED)),
        },
    }
}

/// Handle "image.import" RPC call with streaming progress (admin-only)
///
/// Expected params: ImportRequest {"model": "string", "version": "string", "src": "string", "token": "string"}
async fn handle_image_import_streaming(
    id: String,
    params: serde_json::Value,
    state: &AppState,
    connection: &Arc<Connection>,
    auth_ctx: AuthContext,
) {
    // Parse params into ImportRequest
    let request: data::ImportRequest = match serde_json::from_value(params) {
        Ok(req) => req,
        Err(e) => {
            let response = ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_IMPORT.to_string(),
                    context: Some(format!("{:?}", e)),
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

    // Call import service with progress
    let result = import::import_image(request, state, progress).await;

    // Wait for forward task to complete (channel closes when progress_tx is dropped)
    let _ = forward_task.await;

    // Send final RPC response
    let response = match result {
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
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_IMAGE_IMPORT_FAILED)),
        },
    };

    if let Ok(json) = serde_json::to_string(&response) {
        let _ = connection.send(Message::Text(json.into())).await;
    }
}

/// Handle "image.delete" RPC call
///
/// Expected params: DeleteImageRequest {"model": "string", "version": "string", "token": "string"}
async fn handle_image_delete(
    id: String,
    params: serde_json::Value,
    state: &AppState,
    auth_ctx: AuthContext,
) -> ServerMessage {
    // Parse params into DeleteImageRequest
    let request: data::DeleteImageRequest = match serde_json::from_value(params) {
        Ok(req) => req,
        Err(e) => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_IMAGE_DELETE.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Call delete service
    match delete::delete_image(request, state).await {
        Ok(response) => match serde_json::to_value(&response) {
            Ok(result) => {
                tracing::info!(
                    "Admin '{}' deleted image successfully (model={}, version={})",
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
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_IMAGE_DELETE_FAILED)),
        },
    }
}

/// Handle "image.set_default" RPC call
///
/// Expected params: SetDefaultImageRequest {"model": "string", "version": "string", "token": "string"}
async fn handle_image_set_default(
    id: String,
    params: serde_json::Value,
    state: &AppState,
    auth_ctx: AuthContext,
) -> ServerMessage {
    // Parse params into SetDefaultImageRequest
    let request: data::SetDefaultImageRequest = match serde_json::from_value(params) {
        Ok(req) => req,
        Err(e) => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_IMAGE_SET_DEFAULT.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Call service
    match import::set_default_image(request, state).await {
        Ok(response) => match serde_json::to_value(&response) {
            Ok(result) => {
                tracing::info!(
                    "Admin '{}' set default image successfully (model={}, version={})",
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
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_IMAGE_SET_DEFAULT_FAILED)),
        },
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
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_IMAGE_LIST_FAILED)),
        },
    }
}

/// Handle "image.show" RPC call
///
/// Expected params: ShowImageRequest {"model": "string", "version": "string"?, "token": "string"}
async fn handle_image_show(
    id: String,
    params: serde_json::Value,
    state: &AppState,
) -> ServerMessage {
    // Authenticate the request
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for image.show: {}", e);
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

    // Parse params into ShowImageRequest
    let request: data::ShowImageRequest = match serde_json::from_value(params) {
        Ok(req) => req,
        Err(e) => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_IMAGE_SHOW.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // Call show image service
    match import::show_image(request, state).await {
        Ok(response) => match serde_json::to_value(&response) {
            Ok(result) => {
                tracing::info!(
                    "User '{}' showed image details (model={})",
                    auth_ctx.username,
                    response.image.model
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
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_IMAGE_SHOW_FAILED)),
        },
    }
}

/// Handle "image.scan" RPC call (admin-only)
///
/// Expected params: ScanImagesRequest {"kind": "string"?, "token": "string"}
async fn handle_image_scan(
    id: String,
    params: serde_json::Value,
    state: &AppState,
    auth_ctx: AuthContext,
) -> ServerMessage {
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
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_IMAGE_SCAN_FAILED)),
        },
    }
}

/// Handle "image.pull" RPC call (streaming)
///
/// Expected params: ContainerPullRequest {"model": "string", "repo": "string", "tag": "string", "token": "string"}
async fn handle_image_pull_streaming(
    id: String,
    params: serde_json::Value,
    state: &AppState,
    connection: &Arc<Connection>,
    auth_ctx: AuthContext,
) {
    // Parse params into ContainerPullRequest
    let request: data::ContainerPullRequest = match serde_json::from_value(params) {
        Ok(req) => req,
        Err(e) => {
            send_rpc_error(
                connection,
                id,
                RpcErrorCode::InvalidParams,
                RPC_MSG_INVALID_PARAMS_CONTAINER_PULL.to_string(),
                Some(format!("{:?}", e)),
            )
            .await;
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

    // Call container pull service with progress
    let result = container_pull::pull_container_image(request, state, progress).await;

    // Wait for forward task to complete
    let _ = forward_task.await;

    // Send final RPC response
    let response = match result {
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
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_CONTAINER_PULL_FAILED)),
        },
    };

    if let Ok(json) = serde_json::to_string(&response) {
        let _ = connection.send(Message::Text(json.into())).await;
    }
}

/// Handle "image.download" RPC call
///
/// Expected params: DownloadImageRequest {"model": "string", "version": "string", "url": "string?", "default": bool, "token": "string"}
async fn handle_image_download_streaming(
    id: String,
    params: serde_json::Value,
    state: &AppState,
    connection: &Arc<Connection>,
    auth_ctx: AuthContext,
) {
    // Parse params into DownloadImageRequest
    let request: data::DownloadImageRequest = match serde_json::from_value(params) {
        Ok(req) => req,
        Err(e) => {
            send_rpc_error(
                connection,
                id,
                RpcErrorCode::InvalidParams,
                RPC_MSG_INVALID_PARAMS_IMAGE_DOWNLOAD.to_string(),
                Some(format!("{:?}", e)),
            )
            .await;
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

    // Call download service with progress
    let result = import::download_image(request, state, progress).await;

    // Wait for forward task to complete (channel closes when progress_tx is dropped)
    let _ = forward_task.await;

    // Send final RPC response
    let response = match result {
        Ok(response) => match serde_json::to_value(&response) {
            Ok(result) => {
                tracing::info!(
                    "Admin '{}' downloaded image successfully (model={}, version={})",
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
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_IMAGE_DOWNLOAD_FAILED)),
        },
    };

    if let Ok(json) = serde_json::to_string(&response) {
        let _ = connection.send(Message::Text(json.into())).await;
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
    // Authenticate the request
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for up: {}", e);
            send_rpc_error(
                connection,
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
            send_rpc_error(
                connection,
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
            send_rpc_error(
                connection,
                id,
                RpcErrorCode::InvalidParams,
                RPC_MSG_INVALID_PARAMS_MANIFEST.to_string(),
                None,
            )
            .await;
            return;
        }
    };

    // Reject if lab already exists in the database.
    // A DB record means a previous `up` created it and the lab is still active
    // (or a concurrent `up` just created it). The user must destroy first.
    if let Ok(existing_lab) = db::get_lab(&state.db, &lab_id).await {
        tracing::warn!(
            lab_id = %lab_id,
            user = %auth_ctx.username,
            status = %existing_lab.status,
            "Rejected up request: lab already exists in database"
        );
        let msg = format!(
            "Lab already exists with status '{}'. Destroy it before starting a new one.",
            existing_lab.status
        );
        send_rpc_error(connection, id, RpcErrorCode::ServerError, msg, None).await;
        return;
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

    let node_secrets = match params.get("node_secrets") {
        None | Some(serde_json::Value::Null) => BTreeMap::new(),
        Some(value) => match serde_json::from_value(value.clone()) {
            Ok(secrets) => secrets,
            Err(_) => {
                send_rpc_error(
                    connection,
                    id,
                    RpcErrorCode::InvalidParams,
                    "Invalid node secrets field".to_string(),
                    None,
                )
                .await;
                return;
            }
        },
    };

    // Create UpRequest - use authenticated username
    let request = data::UpRequest {
        lab_id,
        manifest: manifest_value,
        tailscale_auth_key: match tailscale_auth_key_param(&params) {
            Ok(key) => key,
            Err(_) => {
                send_rpc_error(
                    connection,
                    id,
                    RpcErrorCode::InvalidParams,
                    RPC_MSG_INVALID_TAILSCALE_KEY.to_string(),
                    None,
                )
                .await;
                return;
            }
        },
        node_secrets,
    };

    // Call the up service
    let result = up::up_lab(&auth_ctx, request, state, progress).await;

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
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_LAB_UP_FAILED)),
        },
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
    match password::verify_password(&login_request.password, &user.password_hash) {
        Ok(true) => {
            // Password is correct, create JWT token

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
    auth_ctx: AuthContext,
) -> ServerMessage {
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
    _params: serde_json::Value,
    state: &AppState,
    auth_ctx: AuthContext,
) -> ServerMessage {
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
                    created_at: u.created_at.as_second(),
                    updated_at: u.updated_at.as_second(),
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
    auth_ctx: AuthContext,
) -> ServerMessage {
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

    // The service enforces admin, self-delete, last-admin and owned-lab rules
    match user_delete::delete_user(&auth_ctx, &request.username, state).await {
        Ok(response) => match serde_json::to_value(&response) {
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
        },
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_USER_DELETE_FAILED)),
        },
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
    if !auth_ctx.is_admin && normalize_username(&request.username) != auth_ctx.username {
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
    let new_password_hash = match password::hash_password(&request.new_password) {
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
    user.updated_at = jiff::Timestamp::now();

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

/// Handle "user.ssh_key.add" RPC call
///
/// Expected params: AddSshKeyRequest {"username": "string", "ssh_key": "string", "token": "string"}
async fn handle_user_ssh_key_add(
    id: String,
    params: serde_json::Value,
    state: &AppState,
) -> ServerMessage {
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for user.ssh_key.add: {}", e);
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

    let request: data::AddSshKeyRequest = match serde_json::from_value(params) {
        Ok(req) => req,
        Err(e) => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_ADD_SSH_KEY.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // The service enforces the self-or-admin rule and validates the key
    match user_ssh_keys::add_ssh_key(&auth_ctx, &request.username, &request.ssh_key, state).await {
        Ok(response) => match serde_json::to_value(&response) {
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
        },
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_SSH_KEY_ADD_FAILED)),
        },
    }
}

/// Handle "user.ssh_key.remove" RPC call
///
/// Expected params: RemoveSshKeyRequest {"username": "string", "ssh_key": "string", "token": "string"}
async fn handle_user_ssh_key_remove(
    id: String,
    params: serde_json::Value,
    state: &AppState,
) -> ServerMessage {
    let auth_ctx = match middleware::authenticate_request(&params, state).await {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!("Authentication failed for user.ssh_key.remove: {}", e);
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

    let request: data::RemoveSshKeyRequest = match serde_json::from_value(params) {
        Ok(req) => req,
        Err(e) => {
            return ServerMessage::RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: RpcErrorCode::InvalidParams,
                    message: RPC_MSG_INVALID_PARAMS_REMOVE_SSH_KEY.to_string(),
                    context: Some(format!("{:?}", e)),
                }),
            };
        }
    };

    // The service enforces the self-or-admin rule
    match user_ssh_keys::remove_ssh_key(&auth_ctx, &request.username, &request.ssh_key, state).await
    {
        Ok(response) => match serde_json::to_value(&response) {
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
        },
        Err(e) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(service_error(&e, RPC_MSG_SSH_KEY_REMOVE_FAILED)),
        },
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
    if !auth_ctx.is_admin && normalize_username(&request.username) != auth_ctx.username {
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
                created_at: user.created_at.as_second(),
                updated_at: user.updated_at.as_second(),
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
