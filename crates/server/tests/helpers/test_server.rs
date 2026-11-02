use anyhow::Result;
use axum::routing::get;
use bollard::Docker;
use dashmap::DashMap;
use libvirt::Qemu;
use shared::data::{
    Config, OtelConfig, ScannerConfig, ServerConnection, TailscaleGatewaySettings, TlsConfig,
    VmProviders, ZtpServer,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

use sherpad::api::build_router;
use sherpad::api::websocket;
use sherpad::daemon::metrics::Metrics;
use sherpad::daemon::state::AppState;

/// Default admin password used for test servers
pub const TEST_ADMIN_PASSWORD: &str = "TestPass123!";
/// Personal admin account used by tests that start labs; the built-in `admin` cannot.
#[allow(dead_code)]
pub const TEST_LAB_ADMIN: &str = "labadmin";
/// SSH public key registered for test users who start labs.
#[allow(dead_code)]
pub const TEST_SSH_PUBLIC_KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJbRB5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5Jh";

/// A test server that runs an in-process instance of sherpad
pub struct TestServer {
    pub addr: SocketAddr,
    /// Used by lab_lifecycle_tests but not all test binaries, triggering a false positive warning.
    #[allow(dead_code)]
    pub db: db::Database,
}

impl TestServer {
    /// Start a new test server with its own in-memory database
    pub async fn start() -> Result<Self> {
        Self::start_with_config(None).await
    }

    /// Start a new test server with optional config overrides
    pub async fn start_with_config(images_dir: Option<String>) -> Result<Self> {
        let db = db::open_in_memory().await?;
        db::apply_schema(&db).await?;
        db::seed_admin_user(&db, TEST_ADMIN_PASSWORD).await?;

        let docker = Docker::connect_with_local_defaults()?;
        let qemu = Qemu::default();

        let config = Config {
            name: "test-server".to_string(),
            server_ipv4: std::net::Ipv4Addr::new(127, 0, 0, 1),
            server_ipv6: None,
            ws_port: 0,
            http_port: 0,
            vm_provider: VmProviders::default(),
            qemu_bin: "/usr/bin/qemu-system-x86_64".to_string(),
            // Use a range that doesn't conflict with host interfaces (enp3s0 uses 172.31.0.0/16)
            management_prefix_ipv4: "10.200.0.0/16".parse()?,
            management_prefix_ipv6: None,
            images_dir: images_dir.unwrap_or_else(|| "/opt/sherpa/images".to_string()),
            containers_dir: "/opt/sherpa/containers".to_string(),
            bins_dir: "/opt/sherpa/bins".to_string(),
            ztp_server: ZtpServer::default(),
            container_images: vec![],
            server_connection: ServerConnection::default(),
            tls: TlsConfig::default(),
            otel: OtelConfig::default(),
            scanner: ScannerConfig::default(),
            tailscale: TailscaleGatewaySettings::default(),
        };

        let jwt_secret: Vec<u8> = (0..32).map(|_| rand::random::<u8>()).collect();

        let state = AppState {
            connections: sherpad::api::websocket::connection::create_registry(),
            db: db.clone(),
            qemu: Arc::new(qemu),
            docker: Arc::new(docker),
            config: Arc::new(config),
            jwt_secret: Arc::new(jwt_secret),
            metrics: Metrics::noop(),
            pending_jobs: Arc::new(DashMap::new()),
        };

        let app = build_router()
            .route("/ws", get(websocket::handler::ws_handler))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;

        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });

        Ok(Self {
            addr,
            db: db.clone(),
        })
    }

    /// Get the WebSocket URL for this server
    pub fn ws_url(&self) -> String {
        format!("ws://{}/ws", self.addr)
    }
}

/// Open a fresh, empty in-memory database with the schema applied and no users. Used by tests
/// that check startup behaviour before the admin account exists.
#[allow(dead_code)]
pub async fn empty_database() -> Result<db::Database> {
    let db = db::open_in_memory().await?;
    db::apply_schema(&db).await?;
    Ok(db)
}
