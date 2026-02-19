use anyhow::{Context, Result};
use bollard::Docker;
use libvirt::Qemu;
use shared::data::Config;
use shared::konst::{SHERPA_DB_NAME, SHERPA_DB_NAMESPACE, SHERPA_DB_PORT, SHERPA_DB_SERVER};
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

use crate::api::websocket::connection::ConnectionRegistry;
use crate::auth::jwt;

/// Application state shared across the server.
///
/// This contains all runtime state needed by handlers, including:
/// - WebSocket connection registry for real-time communication
/// - Database connection (SurrealDB)
/// - libvirt client (for VMs and unikernels)
/// - Docker client (for containers)
/// - Sherpa configuration
/// - JWT secret for authentication
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
    /// JWT secret for token creation and validation
    pub jwt_secret: Arc<Vec<u8>>,
}

impl AppState {
    /// Create a new AppState with all infrastructure connections
    pub async fn new(config: Config) -> Result<Self> {
        // Load or generate JWT secret
        let jwt_secret =
            jwt::load_or_generate_secret().context("Failed to load or generate JWT secret")?;
        tracing::info!("JWT secret loaded successfully");

        // Connect to SurrealDB
        let db = db::connect(
            SHERPA_DB_SERVER,
            SHERPA_DB_PORT,
            SHERPA_DB_NAMESPACE,
            SHERPA_DB_NAME,
        )
        .await
        .context("Failed to connect to SurrealDB")?;

        tracing::info!(
            "Connected to SurrealDB at {}:{}",
            SHERPA_DB_SERVER,
            SHERPA_DB_PORT
        );

        // Apply database schema
        db::apply_schema(&db)
            .await
            .context("Failed to apply database schema")?;
        tracing::debug!("Database schema applied");

        // Seed node configurations
        match db::seed_node_configs(&db).await {
            Ok(count) => {
                if count > 0 {
                    tracing::info!(count = count, "Seeded node configurations");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to seed node configurations");
            }
        }

        // Seed admin user if SHERPA_ADMIN_PASSWORD is set
        if let Ok(admin_password) = std::env::var("SHERPA_ADMIN_PASSWORD") {
            match db::seed_admin_user(&db, &admin_password).await {
                Ok(true) => {
                    tracing::info!("Admin user created successfully");
                    tracing::info!("Default admin username: admin");
                }
                Ok(false) => {
                    tracing::debug!("Admin user seeding skipped (admin user already exists)");
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to create admin user");
                    return Err(e).context("Admin user creation failed");
                }
            }
        } else {
            // Check if admin user exists
            match db::get_user(&db, "admin").await {
                Ok(_) => {
                    tracing::debug!("Admin user exists");
                }
                Err(_) => {
                    tracing::warn!("No admin user exists in database");
                    tracing::warn!(
                        "Set SHERPA_ADMIN_PASSWORD environment variable to create admin user"
                    );
                    tracing::warn!(
                        "Example: SHERPA_ADMIN_PASSWORD='YourSecurePass123!' sherpad start"
                    );
                }
            }
        }

        // Initialize libvirt client
        let qemu = Qemu::default();
        tracing::info!("Initialized libvirt/QEMU client");

        // Initialize Docker client
        let docker =
            Docker::connect_with_local_defaults().context("Failed to connect to Docker daemon")?;

        tracing::info!("Connected to Docker daemon");

        Ok(Self {
            connections: crate::api::websocket::connection::create_registry(),
            db,
            qemu: Arc::new(qemu),
            docker: Arc::new(docker),
            config: Arc::new(config),
            jwt_secret: Arc::new(jwt_secret),
        })
    }
}
