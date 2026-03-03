use std::time::Duration;

use anyhow::{Context, Result};
use clap::Subcommand;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

use shared::data::{ClientConfig, ServerConnection};

mod clean;
mod image;
mod status;
mod user;

use clean::clean;
use image::{ServerImageCommands, image_commands};
use status::status;
use user::{UserCommands, user_commands};

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Subcommand)]
pub enum ServerCommands {
    /// Check if the Sherpa server is reachable
    Status,

    /// User management commands
    User {
        #[command(subcommand)]
        commands: UserCommands,
    },

    /// Image management commands (admin)
    Image {
        #[command(subcommand)]
        commands: ServerImageCommands,
    },

    /// Force clean all resources for a lab (admin-only)
    Clean {
        /// Lab ID to clean
        lab_id: String,
    },
}

/// Run a server subcommand.
pub async fn run_server(
    commands: &ServerCommands,
    verbose: bool,
    output: &OutputFormat,
    server_url: &str,
    config: &ClientConfig,
) -> Result<()> {
    // Setup logging based on verbose flag
    if verbose {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .init();
    }

    let server_connection = config.server_connection.clone();

    match commands {
        ServerCommands::Status => {
            status(server_url, &server_connection).await?;
        }
        ServerCommands::User { commands } => {
            user_commands(commands, server_url, &server_connection, output).await?;
        }
        ServerCommands::Image { commands } => {
            image_commands(commands, server_url, &server_connection, output).await?;
        }
        ServerCommands::Clean { lab_id } => {
            clean(lab_id, server_url, &server_connection).await?;
        }
    }

    Ok(())
}

/// Convenience helper: load token, connect via WebSocket, send an RPC call, close, return typed result.
pub async fn rpc_call<P, R>(
    method: &str,
    params: P,
    server_url: &str,
    server_connection: &ServerConnection,
) -> Result<R>
where
    P: Serialize,
    R: DeserializeOwned,
{
    let token = load_token().context("Not authenticated. Please login first.")?;

    // Inject token into params
    let mut params_value =
        serde_json::to_value(&params).context("Failed to serialize request params")?;

    if let Some(obj) = params_value.as_object_mut() {
        obj.insert("token".to_string(), serde_json::Value::String(token));
    }

    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        Duration::from_secs(30),
        server_connection.clone(),
    );

    let mut rpc_client = ws_client.connect().await?;

    let request = RpcRequest::new(method, params_value);
    let response = rpc_client.call(request).await?;

    let _ = rpc_client.close().await;

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

    let result = response.result.context("No result in response")?;
    let typed_result: R =
        serde_json::from_value(result).context("Failed to deserialize response")?;

    Ok(typed_result)
}
