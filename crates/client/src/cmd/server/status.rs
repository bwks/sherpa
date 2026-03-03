use std::time::Duration;

use anyhow::Result;

use crate::ws_client::WebSocketClient;
use shared::data::ServerConnection;
use shared::util::render_server_status_table;

/// Check if the Sherpa server is listening by attempting a WebSocket connection.
pub async fn status(server_url: &str, server_connection: &ServerConnection) -> Result<()> {
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        Duration::from_secs(5),
        server_connection.clone(),
    );

    let (status, tls) = match ws_client.connect().await {
        Ok(rpc_client) => {
            let tls = if server_url.starts_with("wss://") {
                "enabled"
            } else {
                "disabled"
            };
            let _ = rpc_client.close().await;
            ("online", tls)
        }
        Err(_) => ("offline", "-"),
    };

    println!("\n{}", render_server_status_table(server_url, status, tls));

    Ok(())
}
