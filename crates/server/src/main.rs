mod api;
mod auth;
mod cli;
mod daemon;
mod services;
mod templates;
mod tls;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use daemon::manager::{
    logs_daemon, restart_daemon, run_background_child, start_daemon, status_daemon, stop_daemon,
};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize rustls crypto provider (required for rustls 0.23+)
    // This must be done before any rustls operations
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Check if we're being spawned as a background child
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "--background-child" {
        return run_background_child().await;
    }

    // Parse CLI arguments
    let cli = Cli::parse();

    // Route to appropriate command handler
    match cli.command {
        Commands::Start { foreground } => start_daemon(foreground).await,
        Commands::Stop { force } => stop_daemon(force),
        Commands::Restart { foreground } => restart_daemon(foreground).await,
        Commands::Status => status_daemon(),
        Commands::Logs { follow } => logs_daemon(follow),
    }
}
