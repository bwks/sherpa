use anyhow::{Context, Result};
use axum::routing::get;
use std::fs::OpenOptions;
use std::sync::Arc;

use crate::api::build_router;
use crate::api::websocket;
use crate::daemon::state::AppState;
use shared::konst::{
    SHERPA_BASE_DIR, SHERPA_CONFIG_DIR, SHERPA_CONFIG_FILE, SHERPA_LOG_DIR, SHERPAD_HOST,
    SHERPAD_LOG_FILE, SHERPAD_PORT,
};
use shared::util::load_config;

/// Run the sherpad server
pub async fn run_server(foreground: bool) -> Result<()> {
    // Setup logging based on mode
    if foreground {
        // Foreground mode: log to stdout
        tracing_subscriber::fmt()
            .with_target(false)
            .with_thread_ids(false)
            .init();
    } else {
        // Background mode: log to file
        let log_file = OpenOptions::new().create(true).append(true).open(&format!(
            "{SHERPA_BASE_DIR}/{SHERPA_LOG_DIR}/{SHERPAD_LOG_FILE}"
        ))?;

        // Wrap file in Arc for thread-safe sharing
        let log_file = Arc::new(log_file);

        tracing_subscriber::fmt()
            .with_writer(move || log_file.clone())
            .with_target(false)
            .with_thread_ids(false)
            .with_ansi(false)
            .init();
    }

    tracing::info!("Starting sherpad server");

    // Load configuration
    let config_path = format!("{SHERPA_BASE_DIR}/{SHERPA_CONFIG_DIR}/{SHERPA_CONFIG_FILE}");
    let config = load_config(&config_path)
        .with_context(|| format!("Failed to load config from {}", config_path))?;

    // Create application state (includes db, libvirt, docker connections)
    let state = AppState::new(config)
        .await
        .context("Failed to initialize application state")?;

    // Build the router with REST endpoints
    let app = build_router()
        // Add WebSocket route
        .route("/ws", get(websocket::handler::ws_handler))
        // Attach state to all routes
        .with_state(state);

    // Bind to configured host:port
    let addr = format!("{}:{}", SHERPAD_HOST, SHERPAD_PORT);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("sherpad listening on {}", addr);

    // Setup graceful shutdown signal handler
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("sherpad server stopped");

    Ok(())
}

/// Handle shutdown signals (SIGTERM, SIGINT)
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install CTRL+C signal handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("Received CTRL+C signal");
        },
        _ = terminate => {
            tracing::info!("Received SIGTERM signal");
        },
    }

    tracing::info!("Starting graceful shutdown");
}
