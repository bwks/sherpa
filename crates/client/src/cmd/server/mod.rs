use anyhow::Result;
use clap::Subcommand;

use shared::data::ClientConfig;

mod clean;
mod image;
mod rpc;
mod status;
mod user;

use clean::clean;
use image::{ServerImageCommands, image_commands};
pub use rpc::{rpc_call, rpc_call_streaming};
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
