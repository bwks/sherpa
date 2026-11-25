#![deny(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(not(test), forbid(unsafe_code))]

use std::env;
use std::io;
use std::path::Path;

use anyhow::Result;
use clap::Parser;
use rustls::crypto::aws_lc_rs;
use shared::konst::SHERPA_DB_FILE_PATH;
use sherpad::check_database::check_database;
use sherpad::cli::{Cli, Commands, UserCommands};
use sherpad::daemon::SHERPAD_PID_FILE_PATH;
use sherpad::daemon::manager::{
    logs_daemon, restart_daemon, run_background_child, start_daemon, status_daemon, stop_daemon,
};
use sherpad::doctor::doctor;
use sherpad::host_account::running_account_name;
use sherpad::init::init;
use sherpad::user::unlock;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize rustls crypto provider (required for rustls 0.23+)
    // This must be done before any rustls operations
    let _ = aws_lc_rs::default_provider().install_default();

    // Check if we're being spawned as a background child
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 && args[1] == "--background-child" {
        return run_background_child().await;
    }

    // Parse CLI arguments
    let cli = Cli::parse();

    // Route to appropriate command handler
    match cli.command {
        Commands::Start { foreground } => start_daemon(foreground).await,
        Commands::Stop { force } => stop_daemon(SHERPAD_PID_FILE_PATH, force),
        Commands::Restart { foreground } => restart_daemon(foreground).await,
        Commands::Status => status_daemon(),
        Commands::Logs { follow } => logs_daemon(follow),
        Commands::Init {
            force,
            server_ipv4,
            server_ipv6,
            ws_port,
            http_port,
            management_prefix_ipv4,
            management_prefix_ipv6,
        } => {
            init(
                force,
                server_ipv4,
                server_ipv6,
                ws_port,
                http_port,
                management_prefix_ipv4,
                management_prefix_ipv6,
            )
            .await
        }
        Commands::CheckDatabase => check_database(Path::new(SHERPA_DB_FILE_PATH)).await,
        Commands::Doctor { images } => doctor(images),
        Commands::User {
            command: UserCommands::Unlock { username },
        } => {
            tracing_subscriber::fmt()
                .with_writer(io::stderr)
                .with_target(false)
                .compact()
                .init();
            unlock(
                &running_account_name()?,
                &username,
                Path::new(SHERPA_DB_FILE_PATH),
                SHERPAD_PID_FILE_PATH,
            )
            .await
        }
    }
}
