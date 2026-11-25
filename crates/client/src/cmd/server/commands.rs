use anyhow::Result;
use clap::Subcommand;
use tracing::Level;

use shared::data::ClientConfig;

use super::backup::backup;
use super::clean::clean;
use super::image::{ServerImageCommands, image_commands};
use super::status::status;
use super::user::{UserCommands, user_commands};

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

    /// Back up the server's database to a file on the server (admin-only)
    Backup,
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
            .with_max_level(Level::DEBUG)
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
        ServerCommands::Backup => {
            backup(server_url, &server_connection, output).await?;
        }
    }

    Ok(())
}
