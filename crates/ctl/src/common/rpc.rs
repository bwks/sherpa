use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use shared::data::ServerConnection;
use shared::error::RpcErrorCode;
use shared::tls::TlsConfigBuilder;
use std::time::Duration;
use tokio_tungstenite::Connector;
use uuid::Uuid;

/// JSON-RPC request message
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RpcRequest {
    pub r#type: String,
    pub id: String,
    pub method: String,
    pub params: serde_json::Value,
}

/// JSON-RPC response message
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RpcResponse {
    pub id: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<RpcError>,
}

/// JSON-RPC error
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RpcError {
    pub code: RpcErrorCode,
    pub message: String,
    pub context: Option<String>,
}

/// Simple RPC client for sherpactl
pub struct RpcClient {
    server_url: String,
    timeout: Duration,
    server_connection: ServerConnection,
}

impl RpcClient {
    pub fn new(server_url: String, server_connection: ServerConnection) -> Self {
        Self {
            server_url,
            timeout: Duration::from_secs(30),
            server_connection,
        }
    }

    /// Call an RPC method with the given parameters and return the typed response
    pub async fn call<P, R>(&self, method: &str, params: P, token: Option<String>) -> Result<R>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let is_secure = self.server_url.starts_with("wss://");

        // Connect to WebSocket with appropriate TLS handling
        let (mut ws_stream, _) = if is_secure {
            // Build TLS configuration with trust-on-first-use flow
            let tls_builder = TlsConfigBuilder::new(&self.server_connection);
            let tls_config = tls_builder
                .build_with_trust_flow(&self.server_url)
                .await
                .context("Failed to build TLS configuration")?;

            let connector = Connector::Rustls(tls_config);

            tokio_tungstenite::connect_async_tls_with_config(
                &self.server_url,
                None,
                false,
                Some(connector),
            )
            .await
            .context(format!(
                "Failed to connect to server at {}",
                self.server_url
            ))?
        } else {
            tokio_tungstenite::connect_async(&self.server_url)
                .await
                .context(format!(
                    "Failed to connect to server at {}",
                    self.server_url
                ))?
        };

        // Add token to params if provided
        let mut params_value =
            serde_json::to_value(&params).context("Failed to serialize request params")?;

        if let Some(token) = token
            && let Some(obj) = params_value.as_object_mut()
        {
            obj.insert("token".to_string(), serde_json::Value::String(token));
        }

        // Create RPC request
        let request = RpcRequest {
            r#type: "rpc_request".to_string(),
            id: Uuid::new_v4().to_string(),
            method: method.to_string(),
            params: params_value,
        };

        // Send request
        let request_json =
            serde_json::to_string(&request).context("Failed to serialize RPC request")?;

        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        ws_stream
            .send(Message::Text(request_json))
            .await
            .context("Failed to send RPC request")?;

        // Loop to find the RPC response (skip non-RPC messages like "connected", "ping", etc.)
        let request_id = request.id.clone();
        let response_text = loop {
            let response_msg = tokio::time::timeout(self.timeout, ws_stream.next())
                .await
                .context("Request timed out")?
                .context("Connection closed")?
                .context("Failed to receive response")?;

            // Parse response
            let text = match response_msg {
                Message::Text(text) => text,
                _ => continue, // Skip non-text messages
            };

            // Check if this is an RPC response by looking for the "type" field
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
                && let Some(msg_type) = value.get("type").and_then(|v| v.as_str())
            {
                if msg_type == "rpc_response" {
                    // Check if this is the response to our request
                    if let Some(id) = value.get("id").and_then(|v| v.as_str()) {
                        if id == request_id {
                            break text;
                        } else {
                            continue; // Skip RPC responses with different IDs
                        }
                    }
                } else {
                    continue; // Skip non-RPC messages (connected, ping, log, etc.)
                }
            }
        };

        // Now parse just the RPC response fields (without the "type" field)
        let response: RpcResponse = serde_json::from_str(&response_text).context(format!(
            "Failed to parse RPC response. Raw JSON: {}",
            response_text
        ))?;

        // Check for RPC error
        if let Some(error) = response.error {
            if let Some(ctx) = &error.context {
                anyhow::bail!(
                    "RPC error: {} (code: {})\n  Context: {}",
                    error.message,
                    error.code,
                    ctx
                );
            } else {
                anyhow::bail!("RPC error: {} (code: {})", error.message, error.code);
            }
        }

        // Parse result
        let result = response.result.context("No result in response")?;
        let typed_result: R =
            serde_json::from_value(result).context("Failed to deserialize response")?;

        // Gracefully close the WebSocket connection
        let _ = ws_stream.close(None).await;

        Ok(typed_result)
    }
}
