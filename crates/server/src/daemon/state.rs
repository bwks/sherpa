use anyhow::{Context, Result};
use bollard::Docker;
use libvirt::Qemu;
use shared::data::Config;
use shared::konst::{SHERPA_DB_NAME, SHERPA_DB_NAMESPACE, SHERPA_DB_PORT, SHERPA_DB_SERVER};
use std::sync::Arc;
use surrealdb::engine::remote::ws::Client;
use surrealdb::Surreal;

use crate::api::websocket::connection::ConnectionRegistry;

/// Application state shared across the server.
///
/// This contains all runtime state needed by handlers, including:
/// - WebSocket connection registry for real-time communication
/// - Database connection (SurrealDB)
/// - libvirt client (for VMs and unikernels)
/// - Docker client (for containers)
/// - Sherpa configuration
#[derive(Clone)]
pub struct AppState {
    /// Registry of active WebSocket connections
    pub connections: ConnectionRegistry,
    /// SurrealDB connection
    pub db: Arc<Surreal<Client>>,
    /// libvirt/QEMU client
    pub qemu: Arc<Qemu>,
    /// Docker client
    pub docker: Arc<Docker>,
    /// Sherpa configuration
    pub config: Arc<Config>,
}

impl AppState {
    /// Create a new AppState with all infrastructure connections
    pub async fn new(config: Config) -> Result<Self> {
        // Connect to SurrealDB
        let db = db::connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME)
            .await
            .context("Failed to connect to SurrealDB")?;

        tracing::info!("Connected to SurrealDB at {}:{}", SHERPA_DB_SERVER, SHERPA_DB_PORT);

        // Initialize libvirt client
        let qemu = Qemu::default();
        tracing::info!("Initialized libvirt/QEMU client");

        // Initialize Docker client
        let docker = Docker::connect_with_local_defaults()
            .context("Failed to connect to Docker daemon")?;
        
        tracing::info!("Connected to Docker daemon");

        Ok(Self {
            connections: crate::api::websocket::connection::create_registry(),
            db: Arc::new(db),
            qemu: Arc::new(qemu),
            docker: Arc::new(docker),
            config: Arc::new(config),
        })
    }
}
