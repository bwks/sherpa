use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Subcommand;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

use shared::data::ServerConnection;
use shared::konst::{SHERPA_CONFIG_FILE_PATH, SHERPA_ENV_FILE_PATH};
use shared::util::{build_websocket_url, get_server_url, load_config, read_env_file_value};

mod clean;
mod doctor;
mod image;
mod init;
mod user;

use clean::clean;
use doctor::doctor;
use image::{ServerImageCommands, image_commands};
use init::init;
use user::{UserCommands, user_commands};

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Subcommand)]
pub enum ServerCommands {
    /// Initialise the Sherpa server environment
    Init {
        /// Overwrite existing config and keys
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        force: bool,

        /// SurrealDB password (also reads from SHERPA_DB_PASSWORD env var or /opt/sherpa/env/sherpa.env)
        #[arg(long = "db-pass", env = "SHERPA_DB_PASSWORD")]
        db_password: Option<String>,

        /// Server listen IP address (also reads from SHERPA_SERVER_IP env var or /opt/sherpa/env/sherpa.env)
        #[arg(long = "server-ip", env = "SHERPA_SERVER_IP")]
        server_ip: Option<String>,
    },

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

    /// Fix up server environment
    Doctor {
        /// Set base box permissions to read-only
        #[arg(long, action = clap::ArgAction::SetTrue)]
        boxes: bool,
    },
}

/// Run a server subcommand.
pub async fn run_server(
    commands: &ServerCommands,
    verbose: bool,
    output: &OutputFormat,
    cli_server_url: Option<String>,
    cli_insecure: bool,
) -> Result<()> {
    // Setup logging based on verbose flag
    if verbose {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .init();
    }

    // Resolve server URL from CLI > env > server config > default
    let config = load_config(SHERPA_CONFIG_FILE_PATH).ok();

    let server_url = cli_server_url
        .or_else(get_server_url)
        .or_else(|| {
            config
                .as_ref()
                .and_then(|c| c.server_connection.url.clone())
        })
        .unwrap_or_else(|| {
            config
                .as_ref()
                .map(build_websocket_url)
                .unwrap_or_else(|| "ws://localhost:3030/ws".to_string())
        });

    // Build ServerConnection from config, with CLI overrides
    let mut server_connection = config
        .as_ref()
        .map(|c| c.server_connection.clone())
        .unwrap_or_default();

    if cli_insecure {
        server_connection.insecure = true;
    }

    match commands {
        ServerCommands::Init {
            force,
            db_password,
            server_ip,
        } => {
            let env_file = Path::new(SHERPA_ENV_FILE_PATH);

            let password = match db_password {
                Some(p) => p.clone(),
                None => read_env_file_value(env_file, "SHERPA_DB_PASSWORD").ok_or_else(|| {
                    anyhow::anyhow!(
                        "Database password not provided. Supply it via:\n  \
                             1. --db-pass flag\n  \
                             2. SHERPA_DB_PASSWORD environment variable\n  \
                             3. SHERPA_DB_PASSWORD entry in {}",
                        env_file.display()
                    )
                })?,
            };

            let ip = match server_ip {
                Some(ip) => ip.clone(),
                None => read_env_file_value(env_file, "SHERPA_SERVER_IP")
                    .unwrap_or_else(|| "0.0.0.0".to_string()),
            };

            init(*force, &password, &ip).await?;
        }
        ServerCommands::User { commands } => {
            user_commands(commands, &server_url, &server_connection, output).await?;
        }
        ServerCommands::Image { commands } => {
            image_commands(commands, &server_url, &server_connection, output).await?;
        }
        ServerCommands::Clean { lab_id } => {
            clean(lab_id, &server_url, &server_connection).await?;
        }
        ServerCommands::Doctor { boxes } => {
            doctor(*boxes)?;
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
