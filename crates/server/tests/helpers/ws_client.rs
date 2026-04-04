use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use super::test_server::TestServer;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// WebSocket RPC test client
pub struct TestWsClient {
    sender: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    receiver: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    pub connection_id: String,
    next_id: u64,
}

impl TestWsClient {
    /// Connect to a test server's WebSocket endpoint
    pub async fn connect(server: &TestServer) -> Result<Self> {
        let (ws_stream, _) = connect_async(&server.ws_url())
            .await
            .context("Failed to connect to WebSocket")?;

        let (sender, mut receiver) = ws_stream.split();

        // Read the initial Connected message
        let connected_msg = timeout(DEFAULT_TIMEOUT, receiver.next())
            .await
            .context("Timeout waiting for Connected message")?
            .context("Stream ended before Connected message")?
            .context("WebSocket error reading Connected message")?;

        let connection_id = match connected_msg {
            Message::Text(text) => {
                let msg: Value =
                    serde_json::from_str(&text).context("Failed to parse Connected message")?;
                msg.get("connection_id")
                    .and_then(|v| v.as_str())
                    .context("Missing connection_id in Connected message")?
                    .to_string()
            }
            other => bail!("Expected text message, got: {:?}", other),
        };

        Ok(Self {
            sender,
            receiver,
            connection_id,
            next_id: 1,
        })
    }

    /// Send an RPC request and wait for the response
    pub async fn rpc_call(&mut self, method: &str, params: Value) -> Result<Value> {
        let (_, response) = self
            .rpc_call_streaming(method, params, DEFAULT_TIMEOUT)
            .await?;
        Ok(response)
    }

    /// Send an RPC request and collect status messages + final response.
    ///
    /// Use this for operations that emit streaming status messages (e.g. lab up/destroy).
    /// For simple request/response RPCs, use `rpc_call` instead.
    pub async fn rpc_call_streaming(
        &mut self,
        method: &str,
        params: Value,
        dur: Duration,
    ) -> Result<(Vec<Value>, Value)> {
        let id = format!("test-{}", self.next_id);
        self.next_id += 1;

        let request = json!({
            "type": "rpc_request",
            "id": id,
            "method": method,
            "params": params,
        });

        self.sender
            .send(Message::Text(serde_json::to_string(&request)?.into()))
            .await
            .context("Failed to send RPC request")?;

        let mut status_messages = Vec::new();

        let result = timeout(dur, async {
            loop {
                let msg = self
                    .receiver
                    .next()
                    .await
                    .context("Stream ended")?
                    .context("WebSocket error")?;

                if let Message::Text(text) = msg {
                    let parsed: Value = serde_json::from_str(&text)?;
                    let msg_type = parsed.get("type").and_then(|v| v.as_str()).unwrap_or("");

                    match msg_type {
                        "status" => {
                            status_messages.push(parsed);
                        }
                        "rpc_response" => {
                            let resp_id = parsed.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            if resp_id == id {
                                return Ok::<(Vec<Value>, Value), anyhow::Error>((
                                    status_messages,
                                    parsed,
                                ));
                            }
                        }
                        _ => {
                            status_messages.push(parsed);
                        }
                    }
                }
            }
        })
        .await
        .context(format!(
            "Timeout waiting for RPC response (connection: {})",
            self.connection_id
        ))??;

        Ok(result)
    }

    /// Login as admin and return the token
    pub async fn login_admin(&mut self) -> Result<String> {
        self.login("admin", super::test_server::TEST_ADMIN_PASSWORD)
            .await
    }

    /// Login with credentials and return the token
    pub async fn login(&mut self, username: &str, password: &str) -> Result<String> {
        let response = self
            .rpc_call(
                "auth.login",
                json!({
                    "username": username,
                    "password": password,
                }),
            )
            .await?;

        let token = response
            .get("result")
            .and_then(|r| r.get("token"))
            .and_then(|t| t.as_str())
            .context("Missing token in login response")?
            .to_string();

        Ok(token)
    }
}
