use anyhow::{Context, Result};
use axum::routing::get;
use std::fs::OpenOptions;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
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
use shared::konst::{SHERPA_CONFIG_FILE_PATH, SHERPAD_LOG_FILE_PATH};
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
        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(SHERPAD_LOG_FILE_PATH)?;

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
    let mut config = load_config(SHERPA_CONFIG_FILE_PATH)
        .context("Failed to load sherpa.toml config - cannot start server")?;

    // Allow SHERPA_SERVER_IPV4 env var to override the config file value
    if let Ok(ip_str) = std::env::var("SHERPA_SERVER_IPV4") {
        match ip_str.parse() {
            Ok(ip) => {
                tracing::info!(
                    "Overriding server IP from SHERPA_SERVER_IPV4 env var: {}",
                    ip_str
                );
                config.server_ipv4 = ip;
            }
            Err(e) => {
                tracing::warn!(
                    "Invalid SHERPA_SERVER_IPV4 '{}': {} — using config value {}",
                    ip_str,
                    e,
                    config.server_ipv4
                );
            }
        }
    }

    // Allow SHERPA_SERVER_IPV6 env var to override the config file value
    if let Ok(ip_str) = std::env::var("SHERPA_SERVER_IPV6") {
        match ip_str.parse::<Ipv6Addr>() {
            Ok(ip) => {
                tracing::info!(
                    "Overriding server IPv6 from SHERPA_SERVER_IPV6 env var: {}",
                    ip_str
                );
                config.server_ipv6 = Some(ip);
            }
            Err(e) => {
                tracing::warn!("Invalid SHERPA_SERVER_IPV6 '{}': {} — ignoring", ip_str, e,);
            }
        }
    }

    // Allow SHERPA_SERVER_WS_PORT env var to override the config file value
    if let Ok(port_str) = std::env::var("SHERPA_SERVER_WS_PORT") {
        match port_str.parse::<u16>() {
            Ok(port) => {
                tracing::info!(
                    "Overriding server port from SHERPA_SERVER_WS_PORT env var: {}",
                    port
                );
                config.ws_port = port;
            }
            Err(e) => {
                tracing::warn!(
                    "Invalid SHERPA_SERVER_WS_PORT '{}': {} — using config value {}",
                    port_str,
                    e,
                    config.ws_port
                );
            }
        }
    }

    // Allow SHERPA_SERVER_HTTP_PORT env var to override the config file value
    if let Ok(port_str) = std::env::var("SHERPA_SERVER_HTTP_PORT") {
        match port_str.parse::<u16>() {
            Ok(port) => {
                tracing::info!(
                    "Overriding HTTP port from SHERPA_SERVER_HTTP_PORT env var: {}",
                    port
                );
                config.http_port = port;
            }
            Err(e) => {
                tracing::warn!(
                    "Invalid SHERPA_SERVER_HTTP_PORT '{}': {} — using config value {}",
                    port_str,
                    e,
                    config.http_port
                );
            }
        }
    }

    // Log server configuration
    let protocol = if config.tls.enabled { "wss" } else { "ws" };
    tracing::info!(
        "Server will listen on {}://{}:{}",
        protocol,
        config.server_ipv4,
        config.ws_port
    );
    if let Some(ipv6) = config.server_ipv6 {
        tracing::info!(
            "Server will also listen on {}://[{}]:{}",
            protocol,
            ipv6,
            config.ws_port
        );
    }

    if config.tls.enabled {
        tracing::info!("TLS is enabled for secure WebSocket connections");
    } else {
        tracing::warn!("TLS is DISABLED - connections will NOT be encrypted!");
    }

    // Create application state (includes db, libvirt, docker connections)
    let state = AppState::new(config.clone())
        .await
        .context("Failed to initialize application state")?;

    // Clone state for HTTP /cert endpoints before moving into main router
    let http_state = state.clone();
    let ipv6_http_state = state.clone();

    // Build the router with REST endpoints
    let app = build_router()
        // Add WebSocket route
        .route("/ws", get(websocket::handler::ws_handler))
        // Attach state to all routes
        .with_state(state);

    // Socket address for binding
    let addr: SocketAddr = format!("{}:{}", config.server_ipv4, config.ws_port)
        .parse()
        .context("Invalid server IP or port")?;

    // Start server with or without TLS
    if config.tls.enabled {
        // TLS-enabled server
        let cert_mgr =
            CertificateManager::new(&config.tls).context("Failed to create certificate manager")?;

        // Determine SANs from config or auto-detect
        let mut san = config.tls.san.clone();
        if san.is_empty() {
            if config.server_ipv4 == Ipv4Addr::UNSPECIFIED {
                // Listening on 0.0.0.0 — add all non-loopback IPv4 interface addresses
                match shared::util::get_non_loopback_ipv4_addresses() {
                    Ok(addrs) => {
                        for ip in &addrs {
                            san.push(format!("IP:{ip}"));
                        }
                        tracing::info!(
                            "Server listening on 0.0.0.0, auto-detected interface IPs for SANs: {:?}",
                            san
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to enumerate network interfaces: {}. Falling back to 0.0.0.0 SAN",
                            e
                        );
                        san.push(format!("IP:{}", config.server_ipv4));
                    }
                }
            } else {
                san.push(format!("IP:{}", config.server_ipv4));
                tracing::info!(
                    "No SANs configured, using server IP: {}",
                    config.server_ipv4
                );
            }

            // Add IPv6 SANs if configured
            if let Some(ipv6) = config.server_ipv6 {
                if ipv6.is_unspecified() {
                    // Listening on [::] — add all non-loopback IPv6 interface addresses
                    match shared::util::get_non_loopback_ipv6_addresses() {
                        Ok(addrs) => {
                            for ip in &addrs {
                                san.push(format!("IP:{ip}"));
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to enumerate IPv6 interfaces: {}. Adding [::] SAN",
                                e
                            );
                            san.push(format!("IP:{}", ipv6));
                        }
                    }
                } else {
                    san.push(format!("IP:{}", ipv6));
                }
            }

            // Always add localhost and loopback
            san.push("DNS:localhost".to_string());
            san.push("IP:127.0.0.1".to_string());
            san.push("IP:::1".to_string());

            // Add hostname and FQDN
            match shared::util::get_hostname() {
                Ok(hostname) => {
                    san.push(format!("DNS:{hostname}"));
                    // If hostname has no dots, try to resolve FQDN
                    if !hostname.contains('.')
                        && let Some(fqdn) = shared::util::get_fqdn()
                        && fqdn != hostname
                    {
                        san.push(format!("DNS:{fqdn}"));
                        tracing::info!("Added FQDN SAN: {}", fqdn);
                    }
                    tracing::info!("Added hostname SAN: {}", hostname);
                }
                Err(e) => {
                    tracing::warn!("Failed to get hostname: {}, skipping hostname SAN", e);
                }
            }

            tracing::info!("Auto-generated SANs: {:?}", san);
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

        // Start HTTP-only listener for certificate download endpoint
        // This allows clients to fetch the certificate via HTTP before trusting it
        let http_port = config.http_port;
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
                    let addr = listener.local_addr();
                    tracing::info!(
                        "HTTP certificate endpoint available at http://{}:{}/cert",
                        addr.as_ref().map(|a| a.ip().to_string()).unwrap_or_default(),
                        addr.as_ref().map(|a| a.port().to_string()).unwrap_or_default()
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

        // Spawn IPv6 HTTP cert endpoint if configured
        if let Some(ipv6) = config.server_ipv6 {
            let ipv6_http_addr = SocketAddr::new(ipv6.into(), config.http_port);
            let v6_http_state = ipv6_http_state;
            tokio::spawn(async move {
                let http_app = axum::Router::new()
                    .route("/cert", get(crate::api::handlers::get_certificate_handler))
                    .with_state(v6_http_state);

                match tokio::net::TcpListener::bind(&ipv6_http_addr).await {
                    Ok(listener) => {
                        tracing::info!(
                            "IPv6 HTTP certificate endpoint available at http://[{}]/cert",
                            ipv6_http_addr
                        );
                        if let Err(e) = axum::serve(listener, http_app).await {
                            tracing::error!("IPv6 HTTP certificate endpoint error: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to bind IPv6 HTTP listener on [{}]: {}",
                            ipv6_http_addr,
                            e
                        );
                    }
                }
            });
        }

        // Spawn IPv6 TLS listener if configured
        if let Some(ipv6) = config.server_ipv6 {
            let ipv6_addr = SocketAddr::new(ipv6.into(), config.ws_port);
            let ipv6_tls_config = tls_config.clone();
            let ipv6_app = app.clone();
            tokio::spawn(async move {
                tracing::info!("Starting IPv6 TLS listener on wss://[{}]", ipv6_addr);
                if let Err(e) = axum_server::bind_rustls(ipv6_addr, ipv6_tls_config)
                    .serve(ipv6_app.into_make_service())
                    .await
                {
                    tracing::error!("IPv6 TLS listener error: {}", e);
                }
            });
        }

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

        // Spawn IPv6 listener if configured
        if let Some(ipv6) = config.server_ipv6 {
            let ipv6_addr = SocketAddr::new(ipv6.into(), config.ws_port);
            let ipv6_app = app.clone();
            tokio::spawn(async move {
                match tokio::net::TcpListener::bind(&ipv6_addr).await {
                    Ok(listener) => {
                        tracing::info!("sherpad also listening on ws://[{}]", ipv6_addr);
                        if let Err(e) = axum::serve(listener, ipv6_app).await {
                            tracing::error!("IPv6 listener error: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to bind IPv6 listener on [{}]: {}", ipv6_addr, e);
                    }
                }
            });
        }

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
        // SAFETY: Signal handler installation is critical for graceful shutdown.
        // If this fails, the process cannot be stopped gracefully.
        #[allow(clippy::expect_used)]
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install CTRL+C signal handler");
    };

    #[cfg(unix)]
    let terminate = async {
        // SAFETY: Signal handler installation is critical for graceful shutdown.
        #[allow(clippy::expect_used)]
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
