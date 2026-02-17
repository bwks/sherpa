use anyhow::{Context, Result};
use axum::routing::get;
use std::fs::OpenOptions;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::time::FormatTime;

/// Custom time formatter that outputs UTC time with millisecond precision
/// Format: 2026-02-17T00:59:15.920Z
struct MillisecondTime;

impl FormatTime for MillisecondTime {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        let now = jiff::Zoned::now();
        // Format with millisecond precision (3 decimal places)
        // Format: YYYY-MM-DDTHH:MM:SS.sssZ
        write!(w, "{}", now.strftime("%Y-%m-%dT%H:%M:%S.%3fZ"))
    }
}

use crate::api::build_router;
use crate::api::websocket;
use crate::daemon::state::AppState;
use crate::tls::CertificateManager;
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
            .with_timer(MillisecondTime)
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
            .with_timer(MillisecondTime)
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

    // Log server configuration
    let protocol = if config.tls.enabled { "wss" } else { "ws" };
    tracing::info!(
        "Server will listen on {}://{}:{}",
        protocol,
        config.server_ipv4,
        config.server_port
    );

    if config.tls.enabled {
        tracing::info!("TLS is enabled for secure WebSocket connections");
    } else {
        tracing::warn!("TLS is DISABLED - connections will NOT be encrypted!");
    }

    // Create application state (includes db, libvirt, docker connections)
    let state = AppState::new(config.clone())
        .await
        .context("Failed to initialize application state")?;

    // Clone state for HTTP /cert endpoint before moving into main router
    let http_state = state.clone();

    // Build the router with REST endpoints
    let app = build_router()
        // Add WebSocket route
        .route("/ws", get(websocket::handler::ws_handler))
        // Attach state to all routes
        .with_state(state);

    // Socket address for binding
    let addr: SocketAddr = format!("{}:{}", config.server_ipv4, config.server_port)
        .parse()
        .context("Invalid server IP or port")?;

    // Start server with or without TLS
    if config.tls.enabled {
        // TLS-enabled server
        let cert_mgr =
            CertificateManager::new(&config.tls).context("Failed to create certificate manager")?;

        // Determine SANs from config or use server IP as default
        let mut san = config.tls.san.clone();
        if san.is_empty() {
            san.push(format!("IP:{}", config.server_ipv4));
            tracing::info!(
                "No SANs configured, using server IP: {}",
                config.server_ipv4
            );
        }

        // Ensure certificates exist (generate if needed)
        cert_mgr
            .ensure_certificates(&san)
            .await
            .context("Failed to ensure TLS certificates exist")?;

        // Load TLS configuration
        let tls_config = cert_mgr
            .load_server_config()
            .await
            .context("Failed to load TLS server configuration")?;

        tracing::info!("Starting TLS-enabled server on wss://{}", addr);

        // Start HTTP-only listener for certificate download endpoint on port + 1
        // This allows clients to fetch the certificate via HTTP before trusting it
        let http_port = config.server_port + 1;
        let http_addr: SocketAddr = format!("{}:{}", config.server_ipv4, http_port)
            .parse()
            .context("Invalid HTTP server address")?;

        tokio::spawn(async move {
            // Build a minimal router with just the /cert endpoint
            let http_app = axum::Router::new()
                .route("/cert", get(crate::api::handlers::get_certificate_handler))
                .with_state(http_state);

            match tokio::net::TcpListener::bind(&http_addr).await {
                Ok(listener) => {
                    tracing::info!(
                        "HTTP certificate endpoint available at http://{}:{}/cert",
                        listener.local_addr().unwrap().ip(),
                        listener.local_addr().unwrap().port()
                    );

                    if let Err(e) = axum::serve(listener, http_app).await {
                        tracing::error!("HTTP certificate endpoint server error: {}", e);
                    }
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to bind HTTP listener on port {} for /cert endpoint: {}. \
                         Certificate download will not be available.",
                        http_port,
                        e
                    );
                }
            }
        });

        // Use axum-server with TLS for main server
        axum_server::bind_rustls(addr, tls_config)
            .serve(app.into_make_service())
            .await
            .context("Failed to start TLS server")?;
    } else {
        // Plain TCP server (no TLS)
        tracing::warn!("Starting server WITHOUT TLS encryption on ws://{}", addr);

        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .with_context(|| {
                format!(
                    "Failed to bind to {} - ensure the IP address is valid and available",
                    addr
                )
            })?;

        tracing::info!("sherpad listening on ws://{}", addr);

        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .context("Failed to start server")?;
    }

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
