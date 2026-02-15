use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::time::Duration;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

use super::messages::{RpcRequest, RpcResponse};

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// WebSocket client for connecting to sherpad server
pub struct WebSocketClient {
    url: String,
    timeout: Duration,
}

impl WebSocketClient {
    /// Create a new WebSocket client
    pub fn new(url: String, timeout: Duration) -> Self {
        Self { url, timeout }
    }

    /// Connect to the WebSocket server and return an RPC client
    pub async fn connect(&self) -> Result<RpcClient> {
        tracing::debug!("Connecting to WebSocket: {}", self.url);

        // Connect with timeout
        let connect_future = connect_async(&self.url);
        let (ws_stream, _) = tokio::time::timeout(self.timeout, connect_future)
            .await
            .context("Connection timeout")?
            .context("Failed to connect to WebSocket")?;

        tracing::debug!("WebSocket connected successfully");

        let (write, mut read) = ws_stream.split();

        // Read the initial "connected" message from server
        if let Some(msg) = tokio::time::timeout(Duration::from_secs(5), read.next()).await? {
            let msg = msg.context("Failed to read initial message")?;
            if let Message::Text(text) = msg {
                tracing::debug!("Received initial message: {}", text);
                // Parse to verify it's a connected message
                #[derive(Deserialize)]
                struct ConnectedMsg {
                    r#type: String,
                }
                if let Ok(connected) = serde_json::from_str::<ConnectedMsg>(&text) {
                    if connected.r#type != "connected" {
                        tracing::warn!("Expected 'connected' message, got: {}", connected.r#type);
                    }
                }
            }
        }

        Ok(RpcClient { write, read })
    }
}

/// RPC client for making RPC calls over WebSocket
pub struct RpcClient {
    write: futures_util::stream::SplitSink<WsStream, Message>,
    read: futures_util::stream::SplitStream<WsStream>,
}

impl RpcClient {
    /// Send an RPC request and wait for the response
    pub async fn call(&mut self, request: RpcRequest) -> Result<RpcResponse> {
        let request_id = request.id.clone();

        // Serialize and send request
        let request_json =
            serde_json::to_string(&request).context("Failed to serialize request")?;
        tracing::debug!("Sending RPC request: {}", request_json);

        self.write
            .send(Message::Text(request_json))
            .await
            .context("Failed to send RPC request")?;

        // Wait for response
        while let Some(msg) = self.read.next().await {
            let msg = msg.context("Error reading WebSocket message")?;

            match msg {
                Message::Text(text) => {
                    tracing::debug!("Received message: {}", text);

                    // Try to parse as RPC response
                    match serde_json::from_str::<RpcResponse>(&text) {
                        Ok(response) => {
                            // Check if this is the response for our request
                            if response.id == request_id {
                                return Ok(response);
                            } else {
                                tracing::warn!(
                                    "Received response for different request ID: {} (expected: {})",
                                    response.id,
                                    request_id
                                );
                            }
                        }
                        Err(e) => {
                            tracing::debug!("Non-RPC message received: {} (error: {})", text, e);
                            // Could be status, log, or other server message - ignore
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
                    bail!("Server closed connection: {:?}", frame);
                }
                _ => {
                    tracing::trace!("Received other message type");
                }
            }
        }

        bail!("Connection closed before receiving response")
    }

    /// Send a streaming RPC request and handle progress updates
    ///
    /// This method sends an RPC request and processes streaming messages (Status, Log)
    /// before receiving the final RPC response. The callback is invoked for each
    /// progress message received.
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
        tracing::debug!("Sending streaming RPC request: {}", request_json);

        self.write
            .send(Message::Text(request_json))
            .await
            .context("Failed to send RPC request")?;

        // Process messages until we receive the final RPC response
        while let Some(msg) = self.read.next().await {
            let msg = msg.context("Error reading WebSocket message")?;

            match msg {
                Message::Text(text) => {
                    tracing::debug!("Received message: {}", text);

                    // Try to parse as RPC response first
                    match serde_json::from_str::<RpcResponse>(&text) {
                        Ok(response) => {
                            // Check if this is the response for our request
                            if response.id == request_id {
                                return Ok(response);
                            } else {
                                tracing::warn!(
                                    "Received response for different request ID: {} (expected: {})",
                                    response.id,
                                    request_id
                                );
                            }
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
                    bail!("Server closed connection: {:?}", frame);
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
