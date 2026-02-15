use anyhow::{Context, Result};
use axum::routing::get;
use std::fs::OpenOptions;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::time::UtcTime;

use crate::api::build_router;
use crate::api::websocket;
use crate::daemon::state::AppState;
use shared::konst::{
    SHERPA_BASE_DIR, SHERPA_CONFIG_DIR, SHERPA_CONFIG_FILE, SHERPA_LOG_DIR, SHERPAD_LOG_FILE,
};
use shared::util::load_config;

/// Run the sherpad server
pub async fn run_server(foreground: bool) -> Result<()> {
    // Create env filter with fallback to 'info' level
    let (filter, using_default) = match EnvFilter::try_from_default_env() {
        Ok(filter) => (filter, false),
        Err(_) => (EnvFilter::new("info"), true),
    };

    // Setup logging based on mode
    if foreground {
        // Foreground mode: log to stdout with colors
        tracing_subscriber::fmt()
            .with_timer(UtcTime::rfc_3339())
            .with_env_filter(filter)
            .with_target(false)
            .with_thread_ids(false)
            .compact()
            .init();
    } else {
        // Background mode: log to file without colors
        let log_file = OpenOptions::new().create(true).append(true).open(&format!(
            "{SHERPA_BASE_DIR}/{SHERPA_LOG_DIR}/{SHERPAD_LOG_FILE}"
        ))?;

        // Wrap file in Arc for thread-safe sharing
        let log_file = Arc::new(log_file);

        tracing_subscriber::fmt()
            .with_timer(UtcTime::rfc_3339())
            .with_env_filter(filter)
            .with_writer(move || log_file.clone())
            .with_target(false)
            .with_thread_ids(false)
            .with_ansi(false)
            .compact()
            .init();
    }

    // Inform if using default log level
    if using_default {
        tracing::info!("RUST_LOG not set or invalid, using default 'info' level");
    }

    tracing::info!("Starting sherpad server");

    // Load configuration
    let config_path = format!("{SHERPA_BASE_DIR}/{SHERPA_CONFIG_DIR}/{SHERPA_CONFIG_FILE}");
    let config = load_config(&config_path)
        .context("Failed to load sherpa.toml config - cannot start server")?;

    tracing::info!(
        "Server will listen on {}:{}",
        config.server_ipv4,
        config.server_port
    );

    // Create application state (includes db, libvirt, docker connections)
    let state = AppState::new(config.clone())
        .await
        .context("Failed to initialize application state")?;

    // Build the router with REST endpoints
    let app = build_router()
        // Add WebSocket route
        .route("/ws", get(websocket::handler::ws_handler))
        // Attach state to all routes
        .with_state(state);

    // Bind to configured host:port
    let addr = format!("{}:{}", config.server_ipv4, config.server_port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| {
            format!(
                "Failed to bind to {} - ensure the IP address is valid and available",
                addr
            )
        })?;

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
