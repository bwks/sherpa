use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
};
use futures_util::StreamExt;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use uuid::Uuid;

use super::connection::Connection;
use super::messages::{ClientMessage, ServerMessage};
use crate::daemon::state::AppState;

/// WebSocket upgrade handler
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

/// Handle individual WebSocket connection
async fn handle_socket(socket: WebSocket, state: AppState) {
    let conn_id = Uuid::new_v4();
    tracing::info!("WebSocket connection established: {}", conn_id);

    let (sender, mut receiver) = socket.split();

    // Create connection and register it
    let connection = Arc::new(Connection::new(conn_id, sender));
    state.connections.insert(conn_id, connection.clone());

    // Send connected confirmation
    let connected_msg = ServerMessage::Connected {
        connection_id: conn_id.to_string(),
    };

    if let Ok(json) = serde_json::to_string(&connected_msg) {
        if let Err(e) = connection.send(Message::Text(json.into())).await {
            tracing::error!("Failed to send connected message: {}", e);
        }
    }

    // Handle incoming messages
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if let Err(e) = handle_client_message(&text, &connection, &state).await {
                    tracing::error!("Error handling message from {}: {}", conn_id, e);

                    // Send error response to client
                    let error_msg = ServerMessage::Result {
                        success: false,
                        message: format!("Error processing message: {}", e),
                        data: None,
                    };

                    if let Ok(json) = serde_json::to_string(&error_msg) {
                        let _ = connection.send(Message::Text(json.into())).await;
                    }
                }
            }
            Ok(Message::Close(_)) => {
                tracing::info!("Client {} closed connection", conn_id);
                break;
            }
            Ok(Message::Ping(data)) => {
                if let Err(e) = connection.send(Message::Pong(data)).await {
                    tracing::error!("Failed to send pong to {}: {}", conn_id, e);
                }
            }
            Ok(Message::Pong(_)) => {
                // Pong received - keepalive working
                tracing::trace!("Received pong from {}", conn_id);
            }
            Ok(Message::Binary(_)) => {
                tracing::warn!("Received unexpected binary message from {}", conn_id);
            }
            Err(e) => {
                tracing::error!("WebSocket error for {}: {}", conn_id, e);
                break;
            }
        }
    }

    // Clean up connection
    state.connections.remove(&conn_id);
    tracing::info!("WebSocket connection closed: {}", conn_id);
}

/// Handle messages from client
async fn handle_client_message(
    text: &str,
    connection: &Arc<Connection>,
    state: &AppState,
) -> Result<(), anyhow::Error> {
    let message: ClientMessage = serde_json::from_str(text)?;

    match message {
        ClientMessage::SubscribeLogs => {
            // Enable log subscription for this connection
            if let Some(conn) = state.connections.get(&connection.id) {
                conn.subscribed_logs.store(true, Ordering::Relaxed);
            }
            tracing::info!("Client {} subscribed to logs", connection.id);

            // Send confirmation
            let response = ServerMessage::Status {
                message: "Subscribed to logs".to_string(),
                timestamp: chrono::Utc::now(),
            };

            if let Ok(json) = serde_json::to_string(&response) {
                connection.send(Message::Text(json.into())).await?;
            }
        }
        ClientMessage::UnsubscribeLogs => {
            if let Some(conn) = state.connections.get(&connection.id) {
                conn.subscribed_logs.store(false, Ordering::Relaxed);
            }
            tracing::info!("Client {} unsubscribed from logs", connection.id);

            // Send confirmation
            let response = ServerMessage::Status {
                message: "Unsubscribed from logs".to_string(),
                timestamp: chrono::Utc::now(),
            };

            if let Ok(json) = serde_json::to_string(&response) {
                connection.send(Message::Text(json.into())).await?;
            }
        }
        ClientMessage::Pong => {
            // Keepalive response - no action needed
            tracing::trace!("Received pong from client {}", connection.id);
        }
        ClientMessage::RpcRequest { id, method, params } => {
            // Handle RPC request asynchronously
            tracing::info!(
                "RPC request received: method={}, id={}, connection={}",
                method,
                id,
                connection.id
            );

            // Clone what we need for the async task
            let connection_id = connection.id;
            let state_clone = state.clone();

            // Spawn async task to handle RPC
            tokio::spawn(async move {
                // Call RPC router
                let response = crate::api::websocket::rpc::handle_rpc_request(
                    id,
                    method,
                    params,
                    &state_clone,
                )
                .await;

                // Send response back to client (get connection from registry)
                if let Some(conn) = state_clone.connections.get(&connection_id) {
                    if let Ok(json) = serde_json::to_string(&response) {
                        if let Err(e) = conn.send(Message::Text(json.into())).await {
                            tracing::error!("Failed to send RPC response: {:?}", e);
                        }
                    }
                }
            });
        }
    }

    Ok(())
}
