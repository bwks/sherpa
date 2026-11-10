use anyhow::{Context, Result};
use axum::Router;
use axum::routing::get;
use opentelemetry::metrics::MeterProvider;
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use std::fs::OpenOptions;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
#[cfg(unix)]
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::api::build_router;
use crate::api::handlers::get_certificate_handler;
use crate::api::websocket;
use crate::auth::session;
use crate::daemon::metrics::Metrics;
use crate::daemon::state::AppState;
use crate::services::scanner::run_scanner;
use crate::tls::CertificateManager;
use shared::data::{Config, OtelConfig};
use shared::konst::{
    OTEL_METRIC_EXPORT_INTERVAL_SECS, SESSION_SWEEP_INTERVAL_SECS, SHERPA_CONFIG_FILE_PATH,
    SHERPAD_LOG_FILE_PATH,
};
use shared::util::{
    get_fqdn, get_hostname, get_non_loopback_ipv4_addresses, get_non_loopback_ipv6_addresses,
    load_config,
};

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

/// Shutdown signal handlers (SIGINT and SIGTERM).
///
/// Installed once, before the server starts listening, so a failure to register a handler stops
/// startup with a clear error instead of leaving a server that cannot shut down gracefully.
struct ShutdownSignals {
    #[cfg(unix)]
    interrupt: Signal,
    #[cfg(unix)]
    terminate: Signal,
}

impl ShutdownSignals {
    /// Register the handlers. Must be called from inside the Tokio runtime.
    #[cfg(unix)]
    fn install() -> Result<Self> {
        let interrupt =
            signal(SignalKind::interrupt()).context("Failed to install SIGINT handler")?;
        let terminate =
            signal(SignalKind::terminate()).context("Failed to install SIGTERM handler")?;
        Ok(Self {
            interrupt,
            terminate,
        })
    }

    #[cfg(not(unix))]
    fn install() -> Result<Self> {
        Ok(Self {})
    }

    /// Wait for the first shutdown signal.
    #[cfg(unix)]
    async fn wait(mut self) {
        tokio::select! {
            _ = self.interrupt.recv() => {
                tracing::info!("Received CTRL+C signal");
            },
            _ = self.terminate.recv() => {
                tracing::info!("Received SIGTERM signal");
            },
        }

        tracing::info!("Starting graceful shutdown");
    }

    #[cfg(not(unix))]
    async fn wait(self) {
        match tokio::signal::ctrl_c().await {
            Ok(()) => tracing::info!("Received CTRL+C signal"),
            Err(e) => tracing::error!("Failed to listen for CTRL+C, shutting down: {}", e),
        }

        tracing::info!("Starting graceful shutdown");
    }
}

/// Holds `OTel` providers so pending data is flushed on drop.
struct OtelGuard {
    tracer_provider: SdkTracerProvider,
    meter_provider: SdkMeterProvider,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        if let Err(e) = self.meter_provider.shutdown() {
            eprintln!("OpenTelemetry meter shutdown error: {e:?}");
        }
        if let Err(e) = self.tracer_provider.shutdown() {
            eprintln!("OpenTelemetry tracer shutdown error: {e:?}");
        }
    }
}

/// What serving the routes needs: the config, the router, the IPv4 address, the token that stops
/// background services, and the shutdown signal handlers.
struct Listening<'a> {
    config: &'a Config,
    app: Router,
    addr: SocketAddr,
    cancel_token: &'a CancellationToken,
    shutdown_signals: ShutdownSignals,
}

/// Resolve the OTLP endpoint, preferring the env var over config.
fn otel_endpoint(otel_config: &OtelConfig) -> String {
    std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").unwrap_or_else(|_| otel_config.endpoint.clone())
}

/// Build the shared `OTel` Resource (service identity) from config / env vars.
fn otel_resource(otel_config: &OtelConfig) -> Resource {
    let service_name =
        std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| otel_config.service_name.clone());
    Resource::builder().with_service_name(service_name).build()
}

/// Build an OTLP tracer provider from config.
fn init_tracer_provider(otel_config: &OtelConfig) -> Result<SdkTracerProvider> {
    let sampler = if (otel_config.sample_ratio - 1.0).abs() < f64::EPSILON {
        opentelemetry_sdk::trace::Sampler::AlwaysOn
    } else if otel_config.sample_ratio <= 0.0 {
        opentelemetry_sdk::trace::Sampler::AlwaysOff
    } else {
        opentelemetry_sdk::trace::Sampler::TraceIdRatioBased(otel_config.sample_ratio)
    };

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(otel_endpoint(otel_config))
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build OTLP span exporter: {e}"))?;

    let provider = SdkTracerProvider::builder()
        .with_sampler(sampler)
        .with_resource(otel_resource(otel_config))
        .with_batch_exporter(exporter)
        .build();

    Ok(provider)
}

/// Build an OTLP meter provider from config.
fn init_meter_provider(otel_config: &OtelConfig) -> Result<SdkMeterProvider> {
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(otel_endpoint(otel_config))
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build OTLP metric exporter: {e}"))?;

    let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(exporter)
        .with_interval(Duration::from_secs(OTEL_METRIC_EXPORT_INTERVAL_SECS))
        .build();

    let provider = SdkMeterProvider::builder()
        .with_resource(otel_resource(otel_config))
        .with_reader(reader)
        .build();

    Ok(provider)
}

/// Run the sherpad server
pub async fn run_server(foreground: bool) -> Result<()> {
    // Load configuration before tracing init so OTel can be configured
    let mut config = load_config(SHERPA_CONFIG_FILE_PATH)
        .context("Failed to load sherpa.toml config - cannot start server")?;

    // The guard flushes pending OTel data when the server stops.
    let (_otel_guard, metrics) = init_telemetry(&config, foreground)?;

    tracing::info!("Starting sherpad server");

    let shutdown_signals =
        ShutdownSignals::install().context("Failed to install shutdown signal handlers")?;

    apply_env_overrides(&mut config);

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
    let state = AppState::new(config.clone(), metrics)
        .await
        .context("Failed to initialize application state")?;

    // Set up shutdown cancellation token for background services
    let cancel_token = CancellationToken::new();

    // Start background scanner service if enabled
    if config.scanner.enabled {
        let scanner_state = state.clone();
        let scanner_token = cancel_token.child_token();
        tokio::spawn(async move {
            run_scanner(scanner_state, scanner_token).await;
        });
    } else {
        tracing::info!("Scanner service is disabled");
    }

    // Delete expired login sessions at startup and then periodically.
    tokio::spawn(session::run_sweeper(
        state.db.clone(),
        cancel_token.child_token(),
        Duration::from_secs(SESSION_SWEEP_INTERVAL_SECS),
    ));

    // Clone state for HTTP /cert endpoints before moving into main router
    let http_state = state.clone();

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
    let listening = Listening {
        config: &config,
        app,
        addr,
        cancel_token: &cancel_token,
        shutdown_signals,
    };
    if config.tls.enabled {
        serve_tls(listening, http_state).await?;
    } else {
        serve_plain(listening).await?;
    }

    // Ensure background services are stopped (idempotent if already cancelled)
    cancel_token.cancel();

    tracing::info!("sherpad server stopped");

    Ok(())
}

/// Set up logging, to the terminal in the foreground or to the log file otherwise, and `OTel`
/// export when it is enabled. Returns the `OTel` guard, which flushes pending data on drop, and
/// the metrics instruments.
fn init_telemetry(config: &Config, foreground: bool) -> Result<(Option<OtelGuard>, Metrics)> {
    // Create env filter with fallback to 'info' level
    let (filter, using_default) = match EnvFilter::try_from_default_env() {
        Ok(filter) => (filter, false),
        Err(_) => (EnvFilter::new("info"), true),
    };

    // Build the optional OTel layer, guard (flushes on drop), and metrics
    let (otel_guard, otel_layer, metrics) = if config.otel.enabled {
        let tracer_provider = init_tracer_provider(&config.otel)?;
        let meter_provider = init_meter_provider(&config.otel)?;

        let tracer = tracer_provider.tracer("sherpad");
        let layer = tracing_opentelemetry::OpenTelemetryLayer::new(tracer);

        let meter = meter_provider.meter("sherpad");
        let metrics = Metrics::new(&meter);

        let guard = OtelGuard {
            tracer_provider,
            meter_provider,
        };
        (Some(guard), Some(layer), metrics)
    } else {
        (None, None, Metrics::noop())
    };

    // Setup logging based on mode using a layered registry.
    // OTel layer is added first (directly on Registry) so its type parameter matches.
    if foreground {
        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_timer(MillisecondTime)
            .with_target(false)
            .with_thread_ids(false)
            .compact();

        tracing_subscriber::registry()
            .with(otel_layer)
            .with(filter)
            .with(fmt_layer)
            .init();
    } else {
        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(SHERPAD_LOG_FILE_PATH)?;

        let log_file = Arc::new(log_file);

        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_timer(MillisecondTime)
            .with_writer(move || log_file.clone())
            .with_target(false)
            .with_thread_ids(false)
            .with_ansi(false)
            .compact();

        tracing_subscriber::registry()
            .with(otel_layer)
            .with(filter)
            .with(fmt_layer)
            .init();
    }

    // Inform if using default log level
    if using_default {
        tracing::info!("RUST_LOG not set or invalid, using default 'info' level");
    }

    if config.otel.enabled {
        tracing::info!(
            "OpenTelemetry trace export enabled, endpoint: {}",
            std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
                .unwrap_or_else(|_| config.otel.endpoint.clone())
        );
    }

    Ok((otel_guard, metrics))
}

/// Let the `SHERPA_SERVER_*` environment variables override the config file's server addresses
/// and ports. An invalid value is logged and the config value kept.
fn apply_env_overrides(config: &mut Config) {
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
}

/// Serve the WebSocket and HTTP routes over TLS, with the plain-HTTP `/cert` endpoint beside
/// them, on IPv4 and, when configured, IPv6, until a shutdown signal.
async fn serve_tls(listening: Listening<'_>, http_state: AppState) -> Result<()> {
    let Listening {
        config,
        app,
        addr,
        cancel_token,
        shutdown_signals,
    } = listening;
    let cert_mgr = CertificateManager::new(&config.tls);

    // Create a handle for graceful shutdown of axum-server
    let handle = axum_server::Handle::new();
    let shutdown_handle = handle.clone();
    let shutdown_token = cancel_token.clone();
    tokio::spawn(async move {
        shutdown_signals.wait().await;
        shutdown_token.cancel();
        shutdown_handle.graceful_shutdown(Some(Duration::from_secs(10)));
    });

    // Ensure certificates exist (generate if needed)
    cert_mgr
        .ensure_certificates(&certificate_sans(config))
        .context("Failed to ensure TLS certificates exist")?;

    // Load TLS configuration
    let tls_config = cert_mgr
        .load_server_config()
        .await
        .context("Failed to load TLS server configuration")?;

    tracing::info!("Starting TLS-enabled server on wss://{}", addr);

    // Start HTTP-only listeners for the certificate download endpoint. They let clients fetch
    // the certificate over HTTP before trusting it.
    let http_addr: SocketAddr = format!("{}:{}", config.server_ipv4, config.http_port)
        .parse()
        .context("Invalid HTTP server address")?;
    tokio::spawn(serve_cert_endpoint(http_addr, http_state.clone()));
    if let Some(ipv6) = config.server_ipv6 {
        tokio::spawn(serve_cert_endpoint(
            SocketAddr::new(ipv6.into(), config.http_port),
            http_state,
        ));
    }

    // Spawn IPv6 TLS listener if configured
    if let Some(ipv6) = config.server_ipv6 {
        let ipv6_addr = SocketAddr::new(ipv6.into(), config.ws_port);
        let ipv6_tls_config = tls_config.clone();
        let ipv6_app = app.clone();
        let ipv6_handle = handle.clone();
        tokio::spawn(async move {
            tracing::info!("Starting IPv6 TLS listener on wss://[{}]", ipv6_addr);
            if let Err(e) = axum_server::bind_rustls(ipv6_addr, ipv6_tls_config)
                .handle(ipv6_handle)
                .serve(ipv6_app.into_make_service())
                .await
            {
                tracing::error!("IPv6 TLS listener error: {}", e);
            }
        });
    }

    // Use axum-server with TLS for main server
    axum_server::bind_rustls(addr, tls_config)
        .handle(handle)
        .serve(app.into_make_service())
        .await
        .context("Failed to start TLS server")
}

/// Serve the WebSocket and HTTP routes without TLS, on IPv4 and, when configured, IPv6, until a
/// shutdown signal.
async fn serve_plain(listening: Listening<'_>) -> Result<()> {
    let Listening {
        config,
        app,
        addr,
        cancel_token,
        shutdown_signals,
    } = listening;
    tracing::warn!("Starting server WITHOUT TLS encryption on ws://{}", addr);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| {
            format!("Failed to bind to {addr} - ensure the IP address is valid and available")
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

    let shutdown_token = cancel_token.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signals.wait().await;
            shutdown_token.cancel();
        })
        .await
        .context("Failed to start server")
}

/// The server certificate's SANs: the configured ones, or else the server's addresses (every
/// interface address when it listens on an unspecified address), localhost and loopback, and the
/// host name and FQDN.
fn certificate_sans(config: &Config) -> Vec<String> {
    let mut san = config.tls.san.clone();
    if !san.is_empty() {
        return san;
    }
    if config.server_ipv4 == Ipv4Addr::UNSPECIFIED {
        // Listening on 0.0.0.0 — add all non-loopback IPv4 interface addresses
        match get_non_loopback_ipv4_addresses() {
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
            match get_non_loopback_ipv6_addresses() {
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
                    san.push(format!("IP:{ipv6}"));
                }
            }
        } else {
            san.push(format!("IP:{ipv6}"));
        }
    }

    // Always add localhost and loopback
    san.push("DNS:localhost".to_string());
    san.push("IP:127.0.0.1".to_string());
    san.push("IP:::1".to_string());

    // Add hostname and FQDN
    match get_hostname() {
        Ok(hostname) => {
            san.push(format!("DNS:{hostname}"));
            // If hostname has no dots, try to resolve FQDN
            if !hostname.contains('.')
                && let Some(fqdn) = get_fqdn()
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
    san
}

/// Serve the plain-HTTP `/cert` endpoint on `addr`. A failure is logged: the TLS server runs on
/// without it, and clients then cannot download the certificate.
async fn serve_cert_endpoint(addr: SocketAddr, state: AppState) {
    // Build a minimal router with just the /cert endpoint
    let http_app = Router::new()
        .route("/cert", get(get_certificate_handler))
        .with_state(state);

    match TcpListener::bind(&addr).await {
        Ok(listener) => {
            match listener.local_addr() {
                Ok(local) => {
                    tracing::info!("HTTP certificate endpoint available at http://{local}/cert");
                }
                Err(e) => {
                    tracing::warn!("Could not determine listener address: {}", e);
                }
            }

            if let Err(e) = axum::serve(listener, http_app).await {
                tracing::error!("HTTP certificate endpoint server error on {addr}: {}", e);
            }
        }
        Err(e) => {
            tracing::error!(
                "Failed to bind HTTP listener on {addr} for /cert endpoint: {}. \
                 Certificate download will not be available.",
                e
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_shutdown_signals_install_inside_runtime() {
        assert!(ShutdownSignals::install().is_ok());
    }

    #[cfg(unix)]
    async fn wait_after_signal(signal_name: &str) {
        let signals = ShutdownSignals::install().unwrap();
        let status = std::process::Command::new("kill")
            .arg(format!("-{signal_name}"))
            .arg(std::process::id().to_string())
            .status()
            .unwrap();
        assert!(status.success());
        tokio::time::timeout(Duration::from_secs(5), signals.wait())
            .await
            .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_shutdown_signals_wait_returns_on_sigterm() {
        wait_after_signal("TERM").await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_shutdown_signals_wait_returns_on_sigint() {
        wait_after_signal("INT").await;
    }
}
