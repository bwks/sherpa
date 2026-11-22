use axum::{
    extract::{
        ConnectInfo, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
};
use futures_util::StreamExt;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tracing::Instrument;
use uuid::Uuid;

use super::connection::Connection;
use super::messages::{ClientMessage, ServerMessage};
use super::rpc;
use crate::api::request_parse::{InvalidRequest, parse_json};
use crate::daemon::state::AppState;
use shared::data::ConnectedMsg;

/// WebSocket upgrade handler. The connection's peer address is the one its logins count against.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state, peer.ip()))
}

/// Handle individual WebSocket connection
async fn handle_socket(socket: WebSocket, state: AppState, peer: IpAddr) {
    let conn_id = Uuid::new_v4();
    let ws_span = tracing::info_span!("ws_connection", connection_id = %conn_id, %peer);
    handle_socket_inner(socket, state, conn_id, peer)
        .instrument(ws_span)
        .await;
}

async fn handle_socket_inner(socket: WebSocket, state: AppState, conn_id: Uuid, peer: IpAddr) {
    tracing::info!("WebSocket connection established: {}", conn_id);

    let (sender, mut receiver) = socket.split();

    // Create connection and register it
    let connection = Arc::new(Connection::new(conn_id, peer, sender));
    state.connections.insert(conn_id, connection.clone());
    state.metrics.ws_connections.add(1, &[]);

    // Send connected confirmation
    let connected_msg = ConnectedMsg {
        connection_id: conn_id.to_string(),
        server_version: env!("CARGO_PKG_VERSION").to_string(),
    };

    if let Ok(json) = serde_json::to_string(&connected_msg)
        && let Err(e) = connection.send(Message::Text(json.into())).await
    {
        tracing::error!("Failed to send connected message: {}", e);
    }

    // Handle incoming messages
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if let Err(e) = handle_client_message(&text, &connection, &state) {
                    tracing::error!("Error handling message from {}: {}", conn_id, e);

                    // Send error response to client
                    let error_msg = ServerMessage::Result {
                        success: false,
                        message: format!("Error processing message: {e}"),
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
    state.metrics.ws_connections.add(-1, &[]);
    tracing::info!("WebSocket connection closed: {}", conn_id);
}

/// Handle messages from client. A message that is not a request is refused with a fixed
/// message naming what is wrong, never what was sent.
fn handle_client_message(
    text: &str,
    connection: &Arc<Connection>,
    state: &AppState,
) -> Result<(), InvalidRequest> {
    let message: ClientMessage = parse_json(text.as_bytes())?;

    match message {
        ClientMessage::RpcRequest { id, method, params } => {
            // Handle RPC request asynchronously
            let rpc_span = tracing::info_span!(
                "rpc_call",
                rpc.method = %method,
                rpc.id = %id,
                connection_id = %connection.id,
            );
            tracing::info!(
                parent: &rpc_span,
                "RPC request received: method={}, id={}, connection={}",
                method,
                id,
                connection.id
            );

            // Clone what we need for the async task
            let connection_clone = Arc::clone(connection);
            let state_clone = state.clone();

            tokio::spawn(
                async move {
                    rpc::dispatch(id, method, params, &state_clone, &connection_clone).await;
                }
                .instrument(rpc_span),
            );
        }
    }

    Ok(())
}
