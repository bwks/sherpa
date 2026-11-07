use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use rustls::ClientConfig;
use shared::data::{ConnectedMsg, ServerConnection};
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, connect_async, connect_async_tls_with_config,
    tungstenite::Message,
};

use super::messages::{RpcRequest, RpcResponse};
use shared::tls::TlsConfigBuilder;

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// How long `RpcClient::call` waits for its response once the request is sent, so a server
/// that stops answering does not hang the command.
const RPC_RESPONSE_TIMEOUT: Duration = Duration::from_mins(15);

/// WebSocket client for connecting to sherpad server
pub struct WebSocketClient {
    url: String,
    timeout: Duration,
    server_connection: ServerConnection,
}

impl WebSocketClient {
    /// Create a new WebSocket client
    pub fn new(url: String, timeout: Duration, server_connection: ServerConnection) -> Self {
        Self {
            url,
            timeout,
            server_connection,
        }
    }

    /// Open the secure WebSocket with `config`, within the connection timeout.
    async fn connect_tls(&self, config: Arc<ClientConfig>) -> Result<WsStream> {
        let connector = Connector::Rustls(config);
        let connect_future = connect_async_tls_with_config(&self.url, None, false, Some(connector));
        let (ws_stream, _) = tokio::time::timeout(self.timeout, connect_future)
            .await
            .context("Connection timeout")?
            .context("Failed to connect to secure WebSocket")?;
        Ok(ws_stream)
    }

    /// Connect to the WebSocket server and return an RPC client
    pub async fn connect(&self) -> Result<RpcClient> {
        tracing::debug!("Connecting to WebSocket: {}", self.url);

        let is_secure = self.url.starts_with("wss://");

        let ws_stream = if is_secure {
            // TLS connection with custom configuration
            tracing::debug!("Using secure WebSocket connection (wss://)");

            // Build TLS configuration with trust-on-first-use flow
            let tls_builder = TlsConfigBuilder::new(&self.server_connection);
            let tls = tls_builder
                .build_with_trust_flow(&self.url)
                .await
                .context("Failed to build TLS configuration")?;

            match self.connect_tls(Arc::clone(&tls.config)).await {
                Ok(ws_stream) => ws_stream,
                Err(error) if tls.refused_unknown_issuer() => {
                    tracing::info!(
                        error = format!("{error:#}"),
                        "No public CA issued the server's certificate; trusting it on first use"
                    );
                    let config = tls_builder
                        .trust_on_first_use(&self.url)
                        .await
                        .context("Failed to build TLS configuration")?;
                    self.connect_tls(config).await?
                }
                Err(error) => return Err(error),
            }
        } else {
            // Plain WebSocket connection
            tracing::debug!("Using plain WebSocket connection (ws://)");
            tracing::warn!("Connection is NOT encrypted - data transmitted in plaintext");

            let connect_future = connect_async(&self.url);
            let (ws_stream, _) = tokio::time::timeout(self.timeout, connect_future)
                .await
                .context("Connection timeout")?
                .context("Failed to connect to WebSocket")?;

            ws_stream
        };

        tracing::debug!("WebSocket connected successfully");

        let (write, mut read) = ws_stream.split();

        // Read the initial "connected" message from server
        if let Some(msg) = tokio::time::timeout(Duration::from_secs(5), read.next()).await? {
            let msg = msg.context("Failed to read initial message")?;
            if let Message::Text(text) = msg {
                tracing::debug!("Received initial message: {}", text);
                // Parse the connected message and check version
                match serde_json::from_str::<ConnectedMsg>(&text) {
                    Ok(msg) => check_version_compatibility(&msg.server_version),
                    Err(e) => tracing::warn!("Failed to parse connected message: {}", e),
                }
            }
        }

        Ok(RpcClient { write, read })
    }
}

/// Check client/server version compatibility and warn on mismatch.
fn check_version_compatibility(server_version: &str) {
    let client_version = env!("CARGO_PKG_VERSION");
    if server_version == client_version {
        tracing::debug!("Client and server versions match: {}", client_version);
    } else {
        eprintln!("WARNING: Version mismatch - client: {client_version}, server: {server_version}");
    }
}

/// Log a message that is not the response being waited for, by size only: messages can carry
/// tokens and credentials.
fn log_other_message(text: &str) {
    tracing::debug!(bytes = text.len(), "Non-RPC message received");
}

/// RPC client for making RPC calls over WebSocket
pub struct RpcClient {
    write: futures_util::stream::SplitSink<WsStream, Message>,
    read: futures_util::stream::SplitStream<WsStream>,
}

impl RpcClient {
    /// Send an RPC request and wait up to `RPC_RESPONSE_TIMEOUT` for its response. Other
    /// messages the server sends meanwhile are only logged.
    pub async fn call(&mut self, request: RpcRequest) -> Result<RpcResponse> {
        let method = request.method.clone();
        tokio::time::timeout(
            RPC_RESPONSE_TIMEOUT,
            self.call_streaming(request, log_other_message),
        )
        .await
        .with_context(|| {
            format!(
                "No response from the server to '{method}' within {} minutes",
                RPC_RESPONSE_TIMEOUT.as_secs() / 60
            )
        })?
    }

    /// Send a streaming RPC request and handle progress updates
    ///
    /// This method sends an RPC request and processes streaming messages (Status, Log)
    /// before receiving the final RPC response. The callback is invoked for each
    /// progress message received. There is no time limit: the server reports progress for as
    /// long as the operation runs, and the user can stop waiting with Ctrl-C.
    pub async fn call_streaming<F>(
        &mut self,
        request: RpcRequest,
        mut callback: F,
    ) -> Result<RpcResponse>
    where
        F: FnMut(&str), // Callback receives message text for parsing
    {
        let request_id = request.id.clone();

        // Serialize and send request
        let request_json =
            serde_json::to_string(&request).context("Failed to serialize request")?;
        tracing::debug!(method = %request.method, id = %request.id, "Sending streaming RPC request");

        self.write
            .send(Message::Text(request_json.into()))
            .await
            .context("Failed to send RPC request")?;

        // Process messages until we receive the final RPC response
        while let Some(msg) = self.read.next().await {
            let msg = msg.context("Error reading WebSocket message")?;

            match msg {
                Message::Text(text) => {
                    // Only the size: responses carry tokens, passwords and kubeconfigs.
                    tracing::debug!(bytes = text.len(), "Received message");

                    // Try to parse as RPC response first
                    match serde_json::from_str::<RpcResponse>(&text) {
                        Ok(response) => {
                            // Check if this is the response for our request
                            if response.id == request_id {
                                return Ok(response);
                            }
                            tracing::warn!(
                                "Received response for different request ID: {} (expected: {})",
                                response.id,
                                request_id
                            );
                        }
                        Err(_) => {
                            // Not an RPC response - likely a Status or Log message
                            // Pass to callback for handling
                            callback(&text);
                        }
                    }
                }
                Message::Ping(_) => {
                    tracing::trace!("Received ping");
                }
                Message::Pong(_) => {
                    tracing::trace!("Received pong");
                }
                Message::Close(frame) => {
                    bail!("Server closed connection: {frame:?}");
                }
                _ => {
                    tracing::trace!("Received other message type");
                }
            }
        }

        bail!("Connection closed before receiving response")
    }

    /// Close the WebSocket connection gracefully
    pub async fn close(mut self) -> Result<()> {
        tracing::debug!("Closing WebSocket connection");
        self.write
            .send(Message::Close(None))
            .await
            .context("Failed to send close frame")?;
        Ok(())
    }
}
