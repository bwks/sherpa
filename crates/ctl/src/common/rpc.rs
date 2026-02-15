use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::time::Duration;
use uuid::Uuid;

// Reuse the WebSocket client from the client crate
use shared::data;

// Import the ws_client types (we'll reference them from client crate)
// This is a bit hacky but avoids duplicating code
// In a real project, this would be in a shared crate

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
    pub r#type: String,
    pub id: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<RpcError>,
}

/// JSON-RPC error
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    pub context: Option<String>,
}

/// Simple RPC client for sherpactl
pub struct RpcClient {
    server_url: String,
    timeout: Duration,
}

impl RpcClient {
    pub fn new(server_url: String) -> Self {
        Self {
            server_url,
            timeout: Duration::from_secs(30),
        }
    }

    /// Call an RPC method with the given parameters and return the typed response
    pub async fn call<P, R>(&self, method: &str, params: P, token: Option<String>) -> Result<R>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        // Connect to WebSocket
        let (mut ws_stream, _) = tokio_tungstenite::connect_async(&self.server_url)
            .await
            .context("Failed to connect to server")?;

        // Add token to params if provided
        let mut params_value = serde_json::to_value(&params)
            .context("Failed to serialize request params")?;
        
        if let Some(token) = token {
            if let Some(obj) = params_value.as_object_mut() {
                obj.insert("token".to_string(), serde_json::Value::String(token));
            }
        }

        // Create RPC request
        let request = RpcRequest {
            r#type: "rpc_request".to_string(),
            id: Uuid::new_v4().to_string(),
            method: method.to_string(),
            params: params_value,
        };

        // Send request
        let request_json = serde_json::to_string(&request)
            .context("Failed to serialize RPC request")?;
        
        use tokio_tungstenite::tungstenite::Message;
        use futures_util::{SinkExt, StreamExt};
        
        ws_stream
            .send(Message::Text(request_json))
            .await
            .context("Failed to send RPC request")?;

        // Wait for response with timeout
        let response_msg = tokio::time::timeout(self.timeout, ws_stream.next())
            .await
            .context("Request timed out")?
            .context("Connection closed")?
            .context("Failed to receive response")?;

        // Parse response
        let response_text = match response_msg {
            Message::Text(text) => text,
            _ => anyhow::bail!("Unexpected message type"),
        };

        let response: RpcResponse = serde_json::from_str(&response_text)
            .context("Failed to parse RPC response")?;

        // Check for RPC error
        if let Some(error) = response.error {
            anyhow::bail!("RPC error: {} (code: {})", error.message, error.code);
        }

        // Parse result
        let result = response.result.context("No result in response")?;
        let typed_result: R = serde_json::from_value(result)
            .context("Failed to deserialize response")?;

        Ok(typed_result)
    }
}
